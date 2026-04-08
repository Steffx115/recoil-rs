//! Skirmish AI with economic planning and tactical awareness.
//!
//! Follows a scripted build order, expands economy, produces army in waves,
//! and attacks when the force is large enough.

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec3};
use pierce_sim::construction::Builder;
use pierce_sim::economy::EconomyState;
use pierce_sim::factory::BuildQueue;
use pierce_sim::unit_defs::UnitDefRegistry;
use pierce_sim::{Allegiance, Dead, MoveState, Position, UnitType};

use crate::building;
use crate::production;
use crate::Lcg;

/// AI decision interval in simulation frames (~3 seconds at 30fps).
const AI_TICK_INTERVAL: u64 = 90;

/// Minimum army size before attacking.
const ATTACK_THRESHOLD: usize = 5;

/// Phases the AI progresses through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiPhase {
    /// Build first factory and initial economy.
    Opening,
    /// Produce units, expand economy.
    Expand,
    /// Accumulated enough army — attack.
    Attack,
}

/// Persistent AI state.
pub struct AiState {
    pub rng: Lcg,
    pub commander: Option<Entity>,
    pub factory: Option<Entity>,
    pub team: u8,
    pub enemy_team: u8,
    pub enemy_commander: Option<Entity>,
    phase: AiPhase,
    /// Metal spots already claimed by this AI (positions).
    claimed_mex_spots: Vec<(f64, f64)>,
    /// Number of solars built.
    solar_count: u32,
    /// Ticks since last attack wave was sent.
    ticks_since_attack: u64,
}

impl AiState {
    pub fn new(
        seed: u64,
        team: u8,
        enemy_team: u8,
        commander: Option<Entity>,
        enemy_commander: Option<Entity>,
    ) -> Self {
        Self {
            rng: Lcg::new(seed),
            commander,
            factory: None,
            team,
            enemy_team,
            enemy_commander,
            phase: AiPhase::Opening,
            claimed_mex_spots: Vec::new(),
            solar_count: 0,
            ticks_since_attack: 0,
        }
    }
}

/// Find the first buildable unit_type_id matching a predicate.
fn find_buildable(
    registry: &UnitDefRegistry,
    builder_type_id: u32,
    predicate: impl Fn(&pierce_sim::unit_defs::UnitDef) -> bool,
) -> Option<u32> {
    let builder_def = registry.get(builder_type_id)?;
    builder_def
        .can_build
        .iter()
        .filter_map(|&id| registry.get(id))
        .find(|d| predicate(d))
        .map(|d| d.unit_type_id)
}

/// Count alive combat units (non-builder, non-building) for a team.
fn count_combat_units(world: &mut World, team: u8) -> usize {
    world
        .query_filtered::<(&Allegiance, &MoveState), (Without<Dead>, Without<Builder>)>()
        .iter(world)
        .filter(|(a, _)| a.team == team)
        .count()
}

/// Run one AI tick. Called every frame; internally checks interval.
pub fn ai_tick(world: &mut World, ai: &mut AiState, frame_count: u64) {
    if !frame_count.is_multiple_of(AI_TICK_INTERVAL) {
        return;
    }

    let cmd = match ai.commander {
        Some(e) if world.get_entity(e).is_ok() && world.get::<Dead>(e).is_none() => e,
        _ => return,
    };

    let cmd_type_id = world.get::<UnitType>(cmd).map(|ut| ut.id).unwrap_or(0);
    let cmd_pos = match world.get::<Position>(cmd) {
        Some(p) => (p.pos.x.to_f32(), p.pos.z.to_f32()),
        None => return,
    };

    // Check factory status.
    let has_factory = ai
        .factory
        .map(|f| world.get_entity(f).is_ok() && world.get::<Dead>(f).is_none())
        .unwrap_or(false);
    if !has_factory {
        ai.factory = None;
    }

    // Check economy.
    let (metal, energy, metal_income) = {
        let economy = world.resource::<EconomyState>();
        economy
            .teams
            .get(&ai.team)
            .map(|r| (r.metal.to_f32(), r.energy.to_f32(), r.metal_income.to_f32()))
            .unwrap_or((0.0, 0.0, 0.0))
    };

    let army_size = count_combat_units(world, ai.team);

    // Update phase.
    ai.phase = if ai.factory.is_none() {
        AiPhase::Opening
    } else if army_size >= ATTACK_THRESHOLD {
        AiPhase::Attack
    } else {
        AiPhase::Expand
    };

    ai.ticks_since_attack += 1;

    let econ = (metal, energy, metal_income);

    match ai.phase {
        AiPhase::Opening => {
            ai_opening(world, ai, cmd, cmd_type_id, cmd_pos);
        }
        AiPhase::Expand => {
            ai_expand(world, ai, cmd, cmd_type_id, cmd_pos, econ);
            ai_produce(world, ai);
        }
        AiPhase::Attack => {
            ai_expand(world, ai, cmd, cmd_type_id, cmd_pos, econ);
            ai_produce(world, ai);
            ai_attack(world, ai);
        }
    }
}

/// Opening: build first factory near commander.
fn ai_opening(
    world: &mut World,
    ai: &mut AiState,
    cmd: Entity,
    cmd_type_id: u32,
    cmd_pos: (f32, f32),
) {
    let factory_id = {
        let registry = world.resource::<UnitDefRegistry>();
        find_buildable(registry, cmd_type_id, |d| d.is_factory())
    };

    if let Some(fid) = factory_id {
        building::place_building(world, Some(cmd), fid, cmd_pos.0 + 40.0, cmd_pos.1, ai.team);
    }

    // Also build a solar while the factory constructs.
    let solar_id = {
        let registry = world.resource::<UnitDefRegistry>();
        find_buildable(registry, cmd_type_id, |d| {
            d.is_building && d.energy_production.is_some() && !d.is_factory()
        })
    };
    if let Some(sid) = solar_id {
        let offset = ai.rng.next_f32(30.0) - 15.0;
        building::place_building(
            world,
            Some(cmd),
            sid,
            cmd_pos.0 + offset,
            cmd_pos.1 - 30.0,
            ai.team,
        );
        ai.solar_count += 1;
    }
}

/// Expand economy: build mexes on metal spots, solars for energy.
fn ai_expand(
    world: &mut World,
    ai: &mut AiState,
    cmd: Entity,
    cmd_type_id: u32,
    cmd_pos: (f32, f32),
    econ: (f32, f32, f32),
) {
    let (_metal, energy, metal_income) = econ;
    // Build mex on unclaimed metal spots.
    let mex_id = {
        let registry = world.resource::<UnitDefRegistry>();
        find_buildable(registry, cmd_type_id, |d| {
            d.is_building && d.metal_production.is_some() && !d.is_factory()
        })
    };

    if let Some(mid) = mex_id {
        // Find nearest unclaimed metal spot.
        let spots = world
            .get_resource::<pierce_sim::map::MetalSpots>()
            .map(|ms| ms.spots.clone())
            .unwrap_or_default();

        let unclaimed = spots.iter().find(|s| {
            !ai.claimed_mex_spots
                .iter()
                .any(|(cx, cz)| (s.x - cx).abs() < 5.0 && (s.z - cz).abs() < 5.0)
        });

        if let Some(spot) = unclaimed {
            let sx = spot.x as f32;
            let sz = spot.z as f32;
            building::place_building(world, Some(cmd), mid, sx, sz, ai.team);
            ai.claimed_mex_spots.push((spot.x, spot.z));
        }
    }

    // Build solar if energy is low or income is low relative to metal income.
    let needs_energy = energy < 200.0 || (metal_income > 2.0 && ai.solar_count < 6);
    if needs_energy {
        let solar_id = {
            let registry = world.resource::<UnitDefRegistry>();
            find_buildable(registry, cmd_type_id, |d| {
                d.is_building && d.energy_production.is_some() && !d.is_factory()
            })
        };
        if let Some(sid) = solar_id {
            let offset = ai.rng.next_f32(60.0) - 30.0;
            building::place_building(
                world,
                Some(cmd),
                sid,
                cmd_pos.0 + offset,
                cmd_pos.1 + ai.rng.next_f32(60.0) - 30.0,
                ai.team,
            );
            ai.solar_count += 1;
        }
    }

    // Build a second factory once we have a stable economy.
    if metal_income > 5.0 && ai.solar_count >= 3 {
        // Check if we only have one factory.
        let factory_count: usize = world
            .query_filtered::<(&BuildQueue, &Allegiance), Without<Dead>>()
            .iter(world)
            .filter(|(_, a)| a.team == ai.team)
            .count();

        if factory_count < 2 {
            let factory_id = {
                let registry = world.resource::<UnitDefRegistry>();
                find_buildable(registry, cmd_type_id, |d| d.is_factory())
            };
            if let Some(fid) = factory_id {
                building::place_building(
                    world,
                    Some(cmd),
                    fid,
                    cmd_pos.0 - 40.0,
                    cmd_pos.1,
                    ai.team,
                );
            }
        }
    }
}

/// Produce units from all factories.
fn ai_produce(world: &mut World, ai: &mut AiState) {
    // Collect all factories.
    let factories: Vec<(Entity, u32)> = world
        .query_filtered::<(Entity, &UnitType, &Allegiance, &BuildQueue), Without<Dead>>()
        .iter(world)
        .filter(|(_, _, a, _)| a.team == ai.team)
        .map(|(e, ut, _, _)| (e, ut.id))
        .collect();

    for (factory, factory_type_id) in factories {
        // Only queue if the factory queue is short.
        let queue_len = world
            .get::<BuildQueue>(factory)
            .map(|bq| bq.queue.len())
            .unwrap_or(0);

        if queue_len >= 3 {
            continue;
        }

        let combat_units: Vec<u32> = {
            let registry = world.resource::<UnitDefRegistry>();
            if let Some(fdef) = registry.get(factory_type_id) {
                fdef.can_build
                    .iter()
                    .filter_map(|&id| registry.get(id))
                    .filter(|d| !d.is_building && !d.is_builder)
                    .map(|d| d.unit_type_id)
                    .collect()
            } else {
                vec![]
            }
        };

        if !combat_units.is_empty() {
            let idx = (ai.rng.next_u32() as usize) % combat_units.len();
            production::queue_unit(world, factory, combat_units[idx]);
        }

        // Track factory.
        if ai.factory.is_none() || ai.factory == Some(factory) {
            ai.factory = Some(factory);
        }
    }
}

/// Attack: send combat units toward the enemy in waves.
fn ai_attack(world: &mut World, ai: &mut AiState) {
    // Only send a wave every ~10 seconds (300 frames) to avoid trickling.
    if ai.ticks_since_attack < 300 / AI_TICK_INTERVAL {
        return;
    }

    let enemy_pos = ai
        .enemy_commander
        .and_then(|e| world.get::<Position>(e))
        .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));

    // If enemy commander is dead, find any enemy unit to target.
    let target_pos = enemy_pos.or_else(|| {
        world
            .query_filtered::<(&Position, &Allegiance), Without<Dead>>()
            .iter(world)
            .find(|(_, a)| a.team == ai.enemy_team)
            .map(|(p, _)| (p.pos.x.to_f32(), p.pos.z.to_f32()))
    });

    let Some((tx, tz)) = target_pos else {
        return;
    };

    let idle_combat: Vec<Entity> = world
        .query_filtered::<(Entity, &Allegiance, &MoveState), (Without<Dead>, Without<Builder>)>()
        .iter(world)
        .filter(|(e, a, ms)| {
            a.team == ai.team && matches!(ms, MoveState::Idle) && Some(*e) != ai.commander
        })
        .map(|(e, _, _)| e)
        .collect();

    if idle_combat.len() >= ATTACK_THRESHOLD {
        for unit in &idle_combat {
            if let Some(ms) = world.get_mut::<MoveState>(*unit) {
                *ms.into_inner() = MoveState::MovingTo(SimVec3::new(
                    SimFloat::from_f32(tx),
                    SimFloat::ZERO,
                    SimFloat::from_f32(tz),
                ));
            }
        }
        ai.ticks_since_attack = 0;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/ai_tests.rs"]
mod tests;
