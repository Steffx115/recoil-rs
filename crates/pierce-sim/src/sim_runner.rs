//! Headless simulation tick runner and determinism test framework.
//!
//! Provides [`sim_tick`] to advance one frame, [`world_checksum`] to
//! compute a deterministic hash of all sim-relevant state, and
//! [`init_sim_world`] to bootstrap a world for simulation.

use bevy_ecs::prelude::*;
use std::hash::{Hash, Hasher};

use crate::collision::collision_system;
use crate::combat_data::DamageTable;
use crate::commands::command_system;
use crate::components::{Heading, Health, MoveState, Position, SimId, Stunned, Velocity};
use crate::damage::{damage_system, stun_system};
use crate::economy::{economy_system, EconomyState};
use crate::factory::{factory_system, UnitRegistry};
use crate::fog::{fog_system, FogOfWar};
use crate::lifecycle::{cleanup_dead, init_lifecycle};
use crate::movement::movement_system;
use crate::footprint::footprint_cleanup_system;
use crate::pathfinding::TerrainGrid;
use crate::projectile::{projectile_movement_system, spawn_projectile_system, ImpactEventQueue};
use crate::spatial::SpatialGrid;
use crate::targeting::{reload_system, targeting_system, FireEventQueue, WeaponRegistry};
use crate::{SimFloat, SimVec2};

/// Run one frame of the simulation in the correct system order.
///
///  1. Rebuild [`SpatialGrid`] from all [`Position`] components.
///  2. [`command_system`]
///  3. [`economy_system`]
///  4. [`movement_system`]
///  5. [`collision_system`]
///  6. [`targeting_system`]
///  7. [`reload_system`]
///  8. [`spawn_projectile_system`]
///  9. [`projectile_movement_system`]
/// 10. [`damage_system`]
/// 11. [`stun_system`]
/// 12. [`fog_system`] (if [`FogOfWar`] resource exists)
/// 13. [`factory_system`] (if [`UnitRegistry`] resource exists)
/// 14. [`footprint_cleanup_system`] (restore terrain for dead buildings)
/// 15. [`cleanup_dead`]
pub fn sim_tick(world: &mut World) {
    // 1. Rebuild spatial grid (exclude Dead entities)
    {
        let entities: Vec<(Entity, SimVec2)> = world
            .query_filtered::<(Entity, &Position), bevy_ecs::query::Without<crate::Dead>>()
            .iter(world)
            .map(|(e, p)| (e, SimVec2::new(p.pos.x, p.pos.z)))
            .collect();

        let mut grid = world.resource_mut::<SpatialGrid>();
        grid.clear();
        for (entity, pos) in entities {
            grid.insert(entity, pos);
        }
    }

    // 2. Command processing
    command_system(world);

    // 3. Economy
    if world.contains_resource::<EconomyState>() {
        economy_system(world);
    }

    // 4. Movement
    movement_system(world);

    // 5. Collision
    collision_system(world);

    // 6. Targeting
    if world.contains_resource::<WeaponRegistry>() {
        targeting_system(world);
    }

    // 7. Reload (weapon cooldowns -> fire events)
    if world.contains_resource::<WeaponRegistry>() {
        reload_system(world);
    }

    // 8. Spawn projectiles from fire events
    if world.contains_resource::<WeaponRegistry>() {
        spawn_projectile_system(world);
    }

    // 9. Projectile movement and impact detection
    if world.contains_resource::<ImpactEventQueue>() {
        projectile_movement_system(world);
    }

    // 10. Damage application
    if world.contains_resource::<ImpactEventQueue>() {
        damage_system(world);
    }

    // 11. Stun tick-down
    stun_system(world);

    // 12. Fog of war (only if resource exists)
    if world.contains_resource::<FogOfWar>() {
        let cell_size = SimFloat::ONE;
        fog_system(world, cell_size);
    }

    // 13. Factory production (only if registry exists)
    if world.contains_resource::<UnitRegistry>() {
        factory_system(world);
    }

    // 14. Restore terrain grid for dead buildings (before despawn)
    footprint_cleanup_system(world);

    // 15. Cleanup dead entities
    cleanup_dead(world);
}

/// Compute a deterministic hash of all sim-relevant state.
///
/// Queries all entities with [`SimId`], sorted by `SimId.id`, and hashes
/// their core components: SimId, Position, Velocity, Heading, Health,
/// MoveState, Stunned.
pub fn world_checksum(world: &mut World) -> u64 {
    let mut entries: Vec<(u64, Entity)> = world
        .query::<(Entity, &SimId)>()
        .iter(world)
        .map(|(e, sid)| (sid.id, e))
        .collect();
    entries.sort_by_key(|&(id, _)| id);

    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    for (_, entity) in entries {
        if let Some(sid) = world.get::<SimId>(entity) {
            sid.hash(&mut hasher);
        }
        if let Some(pos) = world.get::<Position>(entity) {
            pos.hash(&mut hasher);
        }
        if let Some(vel) = world.get::<Velocity>(entity) {
            vel.hash(&mut hasher);
        }
        if let Some(heading) = world.get::<Heading>(entity) {
            heading.hash(&mut hasher);
        }
        if let Some(health) = world.get::<Health>(entity) {
            health.hash(&mut hasher);
        }
        if let Some(ms) = world.get::<MoveState>(entity) {
            ms.hash(&mut hasher);
        }
        if let Some(stunned) = world.get::<Stunned>(entity) {
            stunned.remaining_frames.hash(&mut hasher);
        }
    }

    hasher.finish()
}

/// Initialize a world for simulation with all system resources.
///
/// Inserts lifecycle resources, a [`SpatialGrid`], a [`TerrainGrid`],
/// and combat resources ([`WeaponRegistry`], [`DamageTable`],
/// [`FireEventQueue`], [`ImpactEventQueue`], [`EconomyState`]).
pub fn init_sim_world(world: &mut World) {
    init_lifecycle(world);
    world.insert_resource(SpatialGrid::new(SimFloat::from_int(16), 64, 64));
    world.insert_resource(TerrainGrid::new(64, 64, SimFloat::ONE));

    // Combat resources
    if !world.contains_resource::<WeaponRegistry>() {
        world.insert_resource(WeaponRegistry { defs: Vec::new() });
    }
    if !world.contains_resource::<DamageTable>() {
        world.insert_resource(DamageTable::default());
    }
    if !world.contains_resource::<FireEventQueue>() {
        world.insert_resource(FireEventQueue::default());
    }
    if !world.contains_resource::<ImpactEventQueue>() {
        world.insert_resource(ImpactEventQueue::default());
    }
    if !world.contains_resource::<EconomyState>() {
        world.insert_resource(EconomyState::default());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "sim_runner_tests.rs"]
mod tests;
