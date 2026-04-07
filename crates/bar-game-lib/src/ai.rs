//! Simple AI for controlling a team (e.g., team 1).
//!
//! The AI queries the commander's and factory's `can_build` lists from the
//! [`UnitDefRegistry`] to decide what buildings and units to produce.

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use recoil_math::{SimFloat, SimVec3};
use recoil_sim::unit_defs::UnitDefRegistry;
use recoil_sim::construction::Builder;
use recoil_sim::{Allegiance, Dead, MoveState, Position, UnitType};

use crate::building;
use crate::production;
use crate::Lcg;

/// AI tick interval in simulation frames.
pub const AI_TICK_INTERVAL: u64 = 300;

/// Persistent AI state.
pub struct AiState {
    pub rng: Lcg,
    /// The AI's commander entity.
    pub commander: Option<Entity>,
    /// The AI's first factory entity.
    pub factory: Option<Entity>,
    /// The AI's team.
    pub team: u8,
    /// The enemy team.
    pub enemy_team: u8,
    /// The enemy commander entity (for attack targeting).
    pub enemy_commander: Option<Entity>,
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
        }
    }
}

/// Find the first unit_type_id in a commander/builder's `can_build` list
/// that matches a predicate on the UnitDef.
fn find_buildable(
    registry: &UnitDefRegistry,
    builder_type_id: u32,
    predicate: impl Fn(&recoil_sim::unit_defs::UnitDef) -> bool,
) -> Option<u32> {
    let builder_def = registry.get(builder_type_id)?;
    builder_def
        .can_build
        .iter()
        .filter_map(|&id| registry.get(id))
        .find(|d| predicate(d))
        .map(|d| d.unit_type_id)
}

/// Run one AI tick. Should be called every frame; internally checks the interval.
pub fn ai_tick(world: &mut World, ai: &mut AiState, frame_count: u64) {
    if !frame_count.is_multiple_of(AI_TICK_INTERVAL) {
        return;
    }

    let cmd = match ai.commander {
        Some(e) if world.get_entity(e).is_ok() && world.get::<Dead>(e).is_none() => e,
        _ => return, // AI commander is dead
    };

    // Get the commander's unit_type_id for looking up can_build.
    let cmd_type_id = world.get::<UnitType>(cmd).map(|ut| ut.id).unwrap_or(0);

    // Check if AI has a living factory
    let has_factory = ai.factory.is_some()
        && ai
            .factory
            .map(|f| world.get_entity(f).is_ok() && world.get::<Dead>(f).is_none())
            .unwrap_or(false);

    if !has_factory {
        // Find a factory type in the commander's build list
        let factory_id = {
            let registry = world.resource::<UnitDefRegistry>();
            find_buildable(registry, cmd_type_id, |d| d.is_factory())
        };

        if let Some(fid) = factory_id {
            if let Some(cmd_pos) = world.get::<Position>(cmd) {
                let fx = cmd_pos.pos.x.to_f32() + 40.0;
                let fz = cmd_pos.pos.z.to_f32();
                building::place_building(world, Some(cmd), fid, fx, fz, ai.team);
            }
        }
    } else if let Some(factory) = ai.factory {
        // Get the factory's can_build list and queue a random combat unit
        let factory_type_id = world.get::<UnitType>(factory).map(|ut| ut.id).unwrap_or(0);
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

        // Also build some economy buildings occasionally
        if frame_count.is_multiple_of(AI_TICK_INTERVAL * 3) {
            let solar_id = {
                let registry = world.resource::<UnitDefRegistry>();
                find_buildable(registry, cmd_type_id, |d| {
                    d.is_building && d.energy_production.is_some() && !d.is_factory()
                })
            };
            if let (Some(sid), Some(cmd_pos)) = (solar_id, world.get::<Position>(cmd)) {
                let offset = ai.rng.next_f32(80.0) - 40.0;
                let sx = cmd_pos.pos.x.to_f32() + offset;
                let sz = cmd_pos.pos.z.to_f32() + ai.rng.next_f32(80.0) - 40.0;
                building::place_building(world, Some(cmd), sid, sx, sz, ai.team);
            }
        }
    }

    // Move idle combat units (non-builders) toward enemy commander
    let enemy_pos = ai
        .enemy_commander
        .and_then(|e| world.get::<Position>(e))
        .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));

    if let Some((ex, ez)) = enemy_pos {
        let idle_combat: Vec<Entity> = world
            .query_filtered::<(Entity, &Allegiance, &MoveState), (
                Without<Dead>,
                Without<Builder>,
            )>()
            .iter(world)
            .filter(|(e, a, ms)| {
                a.team == ai.team && matches!(ms, MoveState::Idle) && Some(*e) != ai.commander
            })
            .map(|(e, _, _)| e)
            .collect();

        for unit in idle_combat {
            if let Some(ms) = world.get_mut::<MoveState>(unit) {
                let target = SimVec3::new(
                    SimFloat::from_f32(ex),
                    SimFloat::ZERO,
                    SimFloat::from_f32(ez),
                );
                *ms.into_inner() = MoveState::MovingTo(target);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recoil_sim::economy::init_economy;
    use recoil_sim::sim_runner;

    #[test]
    fn test_ai_tick_no_panic_with_dead_commander() {
        let mut world = World::new();
        sim_runner::init_sim_world(&mut world);
        init_economy(&mut world, &[0, 1]);
        world.insert_resource(UnitDefRegistry::default());

        let mut ai = AiState::new(42, 1, 0, None, None);
        // Should return early without panic
        ai_tick(&mut world, &mut ai, AI_TICK_INTERVAL);
    }
}
