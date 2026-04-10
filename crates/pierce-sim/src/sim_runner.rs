//! Headless simulation tick runner and determinism test framework.

use bevy_ecs::prelude::*;
use std::hash::{Hash, Hasher};

use crate::collision::collision_system;
use crate::commands::command_system;
use crate::components::{Heading, Health, MoveState, Position, SimId, Stunned, Velocity};
use crate::damage::{damage_system, stun_system, DamageableCache};
use crate::economy::economy_system;
use crate::factory::factory_system;
use crate::fog::fog_system_dispatched;
use crate::lifecycle::{cleanup_dead, init_lifecycle};
use crate::movement::movement_system;
use crate::footprint::footprint_cleanup_system;
use crate::pathfinding::TerrainGrid;
use crate::projectile::{projectile_movement_system, spawn_projectile_system};
use crate::shield::{shield_absorb_system, shield_regen_system};
use crate::spatial::SpatialGrid;
use crate::targeting::{reload_system, targeting_system_dispatched};
use crate::{SimFloat, SimVec2};

// Re-export for downstream (init resources).
pub use crate::combat_data::DamageTable;
pub use crate::economy::EconomyState;
pub use crate::factory::UnitRegistry;
pub use crate::projectile::ImpactEventQueue;
pub use crate::targeting::{FireEventQueue, WeaponRegistry};

/// Run one frame of the simulation.
///
/// All resources are expected to be initialized via [`init_sim_world`].
/// Systems are called unconditionally — no runtime capability checks.
/// Fog and targeting dispatch is controlled at compile time via features.
pub fn sim_tick(world: &mut World) {
    // 0. Ensure SimFrameData exists.
    if !world.contains_resource::<crate::frame_data::SimFrameData>() {
        world.insert_resource(crate::frame_data::SimFrameData::default());
    }

    // 1. Collect frame data + rebuild spatial grid (single query pass).
    {
        let mut frame = world.remove_resource::<crate::frame_data::SimFrameData>().unwrap();
        frame.clear();

        let mut query = world.query_filtered::<(
            Entity,
            &Position,
            Option<&crate::components::CollisionRadius>,
            Option<&crate::components::MoveState>,
        ), bevy_ecs::query::Without<crate::Dead>>();

        for (e, p, cr, ms) in query.iter(world) {
            let pos_xz = SimVec2::new(p.pos.x, p.pos.z);
            frame.spatial_entries.push((e, pos_xz));

            if let Some(r) = cr {
                frame.collision_entities.push(crate::frame_data::CollisionData {
                    entity: e,
                    bits: e.to_bits(),
                    pos_x: p.pos.x,
                    pos_z: p.pos.z,
                    radius: r.radius,
                    is_mobile: ms.is_some(),
                });
            }
        }

        let mut grid = world.resource_mut::<SpatialGrid>();
        grid.clear();
        for &(entity, pos) in &frame.spatial_entries {
            grid.insert(entity, pos);
        }

        // Store Arc snapshot for parallel systems.
        frame.grid_snapshot = Some(std::sync::Arc::new(grid.clone()));

        // Store weapon registry snapshot (once — it doesn't change).
        if frame.registry_snapshot.is_none() {
            if let Some(reg) = world.get_resource::<WeaponRegistry>() {
                frame.registry_snapshot = Some(std::sync::Arc::new(reg.clone()));
            }
        }

        world.insert_resource(frame);
    }

    // 2. Command processing
    command_system(world);

    // 3. Economy
    economy_system(world);

    // 4. Shield regeneration
    shield_regen_system(world);

    // 5. Fog of war (compile-time dispatched: inline or GPU)
    if world.contains_resource::<crate::fog::FogOfWar>() {
        let cell_size = SimFloat::ONE;
        fog_system_dispatched(world, cell_size);
    }

    // 6. Movement (compile-time dispatched: scalar or batched)
    movement_system(world);

    // 7. Collision
    collision_system(world);

    // 8. Targeting (compile-time dispatched: inline or GPU)
    targeting_system_dispatched(world);

    // 9. Reload
    reload_system(world);

    // 10. Spawn projectiles
    spawn_projectile_system(world);

    // 11. Shield absorption
    shield_absorb_system(world);

    // 12. Projectile movement
    projectile_movement_system(world);

    // 13. Damage
    damage_system(world);

    // 14. Stun tick-down
    stun_system(world);

    // 15. Factory production
    factory_system(world);

    // 16. Footprint cleanup
    footprint_cleanup_system(world);

    // 17. Cleanup dead
    cleanup_dead(world);
}

/// Compute a deterministic hash of all sim-relevant state.
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
pub fn init_sim_world(world: &mut World) {
    init_sim_world_sized(world, 64, 64, 64, 64, SimFloat::from_int(16));
}

/// Initialize a world for simulation with custom grid dimensions.
pub fn init_sim_world_sized(
    world: &mut World,
    spatial_w: usize,
    spatial_h: usize,
    terrain_w: usize,
    terrain_h: usize,
    terrain_cell_size: SimFloat,
) {
    init_lifecycle(world);
    world.insert_resource(SpatialGrid::new(SimFloat::from_int(16), spatial_w as i32, spatial_h as i32));
    world.insert_resource(TerrainGrid::new(terrain_w, terrain_h, terrain_cell_size));

    // Combat resources — always initialized.
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
    if !world.contains_resource::<DamageableCache>() {
        world.insert_resource(DamageableCache::default());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/sim_runner_tests.rs"]
mod tests;
