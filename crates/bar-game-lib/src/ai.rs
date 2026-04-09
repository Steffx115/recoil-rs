//! Skirmish AI with economic planning and tactical awareness.
//!
//! Follows a scripted build order, expands economy, produces army in waves,
//! and attacks when the force is large enough.
//!
//! The [`AiStrategy`] trait allows pluggable AI behaviors. The default
//! implementation is [`DefaultStrategy`], which mirrors the original hardcoded
//! logic. [`PassiveStrategy`] is a minimal economy-only stub.

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec2, SimVec3};
use pierce_sim::construction::Builder;
use pierce_sim::economy::EconomyState;
use pierce_sim::factory::BuildQueue;
use pierce_sim::unit_defs::UnitDefRegistry;
use pierce_sim::{Allegiance, Dead, MoveState, Position, UnitType};

use crate::building;
use crate::production;
use crate::Lcg;

/// AI decision interval in simulation frames (~3 seconds at 30fps).
pub const AI_TICK_INTERVAL: u64 = 90;

/// Minimum army size before attacking.
const ATTACK_THRESHOLD: usize = 5;

// ---------------------------------------------------------------------------
// AiAction — the vocabulary of things a strategy can request
// ---------------------------------------------------------------------------

/// An action the AI strategy wants to perform.
#[derive(Debug, Clone)]
pub enum AiAction {
    /// Place a building of the given type at map coordinates.
    PlaceBuilding(building::PlacementType, SimVec2),
    /// Queue a unit (by `unit_type_id`) at the given factory entity.
    QueueUnit(Entity, u32),
    /// Send idle combat units toward a target position.
    Attack(SimVec2),
    /// Build a metal extractor at an expansion position.
    Expand(SimVec2),
}

// ---------------------------------------------------------------------------
// AiStrategy trait
// ---------------------------------------------------------------------------

/// A pluggable AI behavior. Implementations inspect the world and AI state,
/// then return a list of [`AiAction`]s to execute.
pub trait AiStrategy: Send + Sync {
    /// Decide what to do this tick. Called once every [`AI_TICK_INTERVAL`]
    /// frames when the commander is alive.
    fn decide(&mut self, world: &mut World, state: &AiState) -> Vec<AiAction>;
}

// ---------------------------------------------------------------------------
// Phases
// ---------------------------------------------------------------------------

/// Phases the AI progresses through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPhase {
    /// Build first factory and initial economy.
    Opening,
    /// Produce units, expand economy.
    Expand,
    /// Accumulated enough army — attack.
    Attack,
}

// ---------------------------------------------------------------------------
// AiState — persistent AI state
// ---------------------------------------------------------------------------

/// Persistent AI state.
pub struct AiState {
    pub rng: Lcg,
    pub commander: Option<Entity>,
    pub factory: Option<Entity>,
    pub team: u8,
    pub enemy_team: u8,
    pub enemy_commander: Option<Entity>,
    pub phase: AiPhase,
    /// Metal spots already claimed by this AI (positions).
    pub claimed_mex_spots: Vec<(f64, f64)>,
    /// Number of solars built.
    pub solar_count: u32,
    /// Ticks since last attack wave was sent.
    pub ticks_since_attack: u64,
    /// Pluggable strategy driving decisions.
    strategy: Box<dyn AiStrategy>,
}

impl AiState {
    pub fn new(
        seed: u64,
        team: u8,
        enemy_team: u8,
        commander: Option<Entity>,
        enemy_commander: Option<Entity>,
    ) -> Self {
        Self::with_strategy(
            seed,
            team,
            enemy_team,
            commander,
            enemy_commander,
            Box::new(DefaultStrategy),
        )
    }

    pub fn with_strategy(
        seed: u64,
        team: u8,
        enemy_team: u8,
        commander: Option<Entity>,
        enemy_commander: Option<Entity>,
        strategy: Box<dyn AiStrategy>,
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
            strategy,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the first buildable unit_type_id matching a predicate.
pub fn find_buildable(
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
pub fn count_combat_units(world: &mut World, team: u8) -> usize {
    world
        .query_filtered::<(&Allegiance, &MoveState), (Without<Dead>, Without<Builder>)>()
        .iter(world)
        .filter(|(a, _)| a.team == team)
        .count()
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

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

    // Check factory status.
    let has_factory = ai
        .factory
        .map(|f| world.get_entity(f).is_ok() && world.get::<Dead>(f).is_none())
        .unwrap_or(false);
    if !has_factory {
        ai.factory = None;
    }

    // Update phase based on game state.
    let army_size = count_combat_units(world, ai.team);
    ai.phase = if ai.factory.is_none() {
        AiPhase::Opening
    } else if army_size >= ATTACK_THRESHOLD {
        AiPhase::Attack
    } else {
        AiPhase::Expand
    };

    ai.ticks_since_attack += 1;

    // --- Ask the strategy what to do ---
    // Safety: we temporarily take the strategy out so `decide` can borrow
    // world mutably while we still own `ai` for reading state.
    let mut strategy = std::mem::replace(&mut ai.strategy, Box::new(NoopStrategy));
    let actions = strategy.decide(world, ai);
    ai.strategy = strategy;

    // --- Execute returned actions ---
    execute_actions(world, ai, cmd, cmd_type_id, &actions);
}

/// Execute a list of AI actions, updating AI state as needed.
fn execute_actions(
    world: &mut World,
    ai: &mut AiState,
    cmd: Entity,
    cmd_type_id: u32,
    actions: &[AiAction],
) {
    for action in actions {
        match action {
            AiAction::PlaceBuilding(placement, pos) => {
                building::place_building(
                    world,
                    Some(cmd),
                    placement.0,
                    pos.x.to_f32(),
                    pos.y.to_f32(),
                    ai.team,
                );
                // Track solar count for energy buildings.
                let registry = world.resource::<UnitDefRegistry>();
                if let Some(def) = registry.get(placement.0) {
                    if def.is_building && def.energy_production.is_some() && !def.is_factory() {
                        ai.solar_count += 1;
                    }
                    if def.is_factory() && ai.factory.is_none() {
                        // We don't know the entity yet (it's being placed),
                        // but factory tracking happens in produce/tick.
                    }
                }
            }
            AiAction::QueueUnit(factory, unit_type_id) => {
                production::queue_unit(world, *factory, *unit_type_id);
                // Track factory.
                if ai.factory.is_none() || ai.factory == Some(*factory) {
                    ai.factory = Some(*factory);
                }
            }
            AiAction::Attack(target) => {
                send_attack_wave(world, ai, *target);
            }
            AiAction::Expand(pos) => {
                let mex_id = {
                    let registry = world.resource::<UnitDefRegistry>();
                    find_buildable(registry, cmd_type_id, |d| {
                        d.is_building && d.metal_production.is_some() && !d.is_factory()
                    })
                };
                if let Some(mid) = mex_id {
                    building::place_building(
                        world,
                        Some(cmd),
                        mid,
                        pos.x.to_f32(),
                        pos.y.to_f32(),
                        ai.team,
                    );
                    ai.claimed_mex_spots
                        .push((pos.x.to_f64(), pos.y.to_f64()));
                }
            }
        }
    }
}

/// Send idle combat units toward a target position.
fn send_attack_wave(world: &mut World, ai: &mut AiState, target: SimVec2) {
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
                    target.x,
                    SimFloat::ZERO,
                    target.y,
                ));
            }
        }
        ai.ticks_since_attack = 0;
    }
}

// ---------------------------------------------------------------------------
// NoopStrategy — internal placeholder used during strategy.decide()
// ---------------------------------------------------------------------------

struct NoopStrategy;

impl AiStrategy for NoopStrategy {
    fn decide(&mut self, _world: &mut World, _state: &AiState) -> Vec<AiAction> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// DefaultStrategy — original hardcoded logic, packaged as a strategy
// ---------------------------------------------------------------------------

/// The original AI behavior: scripted opening, economy expansion, wave attacks.
pub struct DefaultStrategy;

impl AiStrategy for DefaultStrategy {
    fn decide(&mut self, world: &mut World, state: &AiState) -> Vec<AiAction> {
        let cmd = match state.commander {
            Some(e) => e,
            None => return vec![],
        };

        let cmd_type_id = world.get::<UnitType>(cmd).map(|ut| ut.id).unwrap_or(0);
        let cmd_pos = match world.get::<Position>(cmd) {
            Some(p) => (p.pos.x.to_f32(), p.pos.z.to_f32()),
            None => return vec![],
        };

        let (metal, energy, metal_income) = {
            let economy = world.resource::<EconomyState>();
            economy
                .teams
                .get(&state.team)
                .map(|r| (r.metal.to_f32(), r.energy.to_f32(), r.metal_income.to_f32()))
                .unwrap_or((0.0, 0.0, 0.0))
        };

        let mut actions = Vec::new();

        match state.phase {
            AiPhase::Opening => {
                default_opening(world, state, cmd_type_id, cmd_pos, &mut actions);
            }
            AiPhase::Expand => {
                default_expand(
                    world, state, cmd_type_id, cmd_pos,
                    (metal, energy, metal_income),
                    &mut actions,
                );
                default_produce(world, state, &mut actions);
            }
            AiPhase::Attack => {
                default_expand(
                    world, state, cmd_type_id, cmd_pos,
                    (metal, energy, metal_income),
                    &mut actions,
                );
                default_produce(world, state, &mut actions);
                default_attack(world, state, &mut actions);
            }
        }

        actions
    }
}

/// Opening: build first factory near commander.
fn default_opening(
    world: &mut World,
    state: &AiState,
    cmd_type_id: u32,
    cmd_pos: (f32, f32),
    actions: &mut Vec<AiAction>,
) {
    let factory_id = {
        let registry = world.resource::<UnitDefRegistry>();
        find_buildable(registry, cmd_type_id, |d| d.is_factory())
    };

    if let Some(fid) = factory_id {
        actions.push(AiAction::PlaceBuilding(
            building::PlacementType(fid),
            SimVec2::new(
                SimFloat::from_f32(cmd_pos.0 + 40.0),
                SimFloat::from_f32(cmd_pos.1),
            ),
        ));
    }

    // Also build a solar while the factory constructs.
    let solar_id = {
        let registry = world.resource::<UnitDefRegistry>();
        find_buildable(registry, cmd_type_id, |d| {
            d.is_building && d.energy_production.is_some() && !d.is_factory()
        })
    };
    if let Some(sid) = solar_id {
        // Use a deterministic offset derived from solar_count.
        let offset = ((state.solar_count as f32) * 7.3) % 30.0 - 15.0;
        actions.push(AiAction::PlaceBuilding(
            building::PlacementType(sid),
            SimVec2::new(
                SimFloat::from_f32(cmd_pos.0 + offset),
                SimFloat::from_f32(cmd_pos.1 - 30.0),
            ),
        ));
    }
}

/// Expand economy: build mexes on metal spots, solars for energy.
fn default_expand(
    world: &mut World,
    state: &AiState,
    cmd_type_id: u32,
    cmd_pos: (f32, f32),
    econ: (f32, f32, f32),
    actions: &mut Vec<AiAction>,
) {
    let (_metal, energy, metal_income) = econ;

    // Build mex on unclaimed metal spots.
    let spots = world
        .get_resource::<pierce_sim::map::MetalSpots>()
        .map(|ms| ms.spots.clone())
        .unwrap_or_default();

    let unclaimed = spots.iter().find(|s| {
        !state
            .claimed_mex_spots
            .iter()
            .any(|(cx, cz)| (s.x - cx).abs() < 5.0 && (s.z - cz).abs() < 5.0)
    });

    if let Some(spot) = unclaimed {
        actions.push(AiAction::Expand(SimVec2::new(
            SimFloat::from_f64(spot.x),
            SimFloat::from_f64(spot.z),
        )));
    }

    // Build solar if energy is low or income is low relative to metal income.
    let needs_energy = energy < 200.0 || (metal_income > 2.0 && state.solar_count < 6);
    if needs_energy {
        let solar_id = {
            let registry = world.resource::<UnitDefRegistry>();
            find_buildable(registry, cmd_type_id, |d| {
                d.is_building && d.energy_production.is_some() && !d.is_factory()
            })
        };
        if let Some(sid) = solar_id {
            let offset = ((state.solar_count as f32) * 13.7) % 60.0 - 30.0;
            let offset2 = ((state.solar_count as f32) * 23.1) % 60.0 - 30.0;
            actions.push(AiAction::PlaceBuilding(
                building::PlacementType(sid),
                SimVec2::new(
                    SimFloat::from_f32(cmd_pos.0 + offset),
                    SimFloat::from_f32(cmd_pos.1 + offset2),
                ),
            ));
        }
    }

    // Build a second factory once we have a stable economy.
    if metal_income > 5.0 && state.solar_count >= 3 {
        let factory_count: usize = world
            .query_filtered::<(&BuildQueue, &Allegiance), Without<Dead>>()
            .iter(world)
            .filter(|(_, a)| a.team == state.team)
            .count();

        if factory_count < 2 {
            let factory_id = {
                let registry = world.resource::<UnitDefRegistry>();
                find_buildable(registry, cmd_type_id, |d| d.is_factory())
            };
            if let Some(fid) = factory_id {
                actions.push(AiAction::PlaceBuilding(
                    building::PlacementType(fid),
                    SimVec2::new(
                        SimFloat::from_f32(cmd_pos.0 - 40.0),
                        SimFloat::from_f32(cmd_pos.1),
                    ),
                ));
            }
        }
    }
}

/// Produce units from all factories.
fn default_produce(world: &mut World, state: &AiState, actions: &mut Vec<AiAction>) {
    let factories: Vec<(Entity, u32)> = world
        .query_filtered::<(Entity, &UnitType, &Allegiance, &BuildQueue), Without<Dead>>()
        .iter(world)
        .filter(|(_, _, a, _)| a.team == state.team)
        .map(|(e, ut, _, _)| (e, ut.id))
        .collect();

    // Use a simple deterministic index based on solar_count.
    let mut rng_state = state.solar_count.wrapping_mul(2_654_435_761).wrapping_add(1);

    for (factory, factory_type_id) in &factories {
        let queue_len = world
            .get::<BuildQueue>(*factory)
            .map(|bq| bq.queue.len())
            .unwrap_or(0);

        if queue_len >= 3 {
            continue;
        }

        let combat_units: Vec<u32> = {
            let registry = world.resource::<UnitDefRegistry>();
            if let Some(fdef) = registry.get(*factory_type_id) {
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
            let idx = (rng_state as usize) % combat_units.len();
            rng_state = rng_state.wrapping_mul(2_654_435_761).wrapping_add(1);
            actions.push(AiAction::QueueUnit(*factory, combat_units[idx]));
        }
    }
}

/// Attack: send combat units toward the enemy in waves.
fn default_attack(world: &mut World, state: &AiState, actions: &mut Vec<AiAction>) {
    // Only send a wave every ~10 seconds (300 frames) to avoid trickling.
    if state.ticks_since_attack < 300 / AI_TICK_INTERVAL {
        return;
    }

    let enemy_pos = state
        .enemy_commander
        .and_then(|e| world.get::<Position>(e))
        .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));

    // If enemy commander is dead, find any enemy unit to target.
    let target_pos = enemy_pos.or_else(|| {
        world
            .query_filtered::<(&Position, &Allegiance), Without<Dead>>()
            .iter(world)
            .find(|(_, a)| a.team == state.enemy_team)
            .map(|(p, _)| (p.pos.x.to_f32(), p.pos.z.to_f32()))
    });

    if let Some((tx, tz)) = target_pos {
        actions.push(AiAction::Attack(SimVec2::new(
            SimFloat::from_f32(tx),
            SimFloat::from_f32(tz),
        )));
    }
}

// ---------------------------------------------------------------------------
// PassiveStrategy — economy-only stub (no military)
// ---------------------------------------------------------------------------

/// A passive AI that only builds economy (mexes and solars). Never produces
/// combat units or attacks. Useful as a baseline or for testing.
pub struct PassiveStrategy;

impl AiStrategy for PassiveStrategy {
    fn decide(&mut self, world: &mut World, state: &AiState) -> Vec<AiAction> {
        let cmd = match state.commander {
            Some(e) => e,
            None => return vec![],
        };

        let cmd_type_id = world.get::<UnitType>(cmd).map(|ut| ut.id).unwrap_or(0);
        let cmd_pos = match world.get::<Position>(cmd) {
            Some(p) => (p.pos.x.to_f32(), p.pos.z.to_f32()),
            None => return vec![],
        };

        let (_metal, energy, metal_income) = {
            let economy = world.resource::<EconomyState>();
            economy
                .teams
                .get(&state.team)
                .map(|r| (r.metal.to_f32(), r.energy.to_f32(), r.metal_income.to_f32()))
                .unwrap_or((0.0, 0.0, 0.0))
        };

        let mut actions = Vec::new();

        // Always build a factory first (needed for economy flow).
        if state.phase == AiPhase::Opening {
            default_opening(world, state, cmd_type_id, cmd_pos, &mut actions);
        } else {
            // Expand economy only — no produce, no attack.
            default_expand(
                world, state, cmd_type_id, cmd_pos,
                (_metal, energy, metal_income),
                &mut actions,
            );
        }

        actions
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/ai_tests.rs"]
mod tests;
