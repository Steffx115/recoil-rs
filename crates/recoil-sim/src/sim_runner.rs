//! Headless simulation tick runner and determinism test framework.
//!
//! Provides [`sim_tick`] to advance one frame, [`world_checksum`] to
//! compute a deterministic hash of all sim-relevant state, and
//! [`init_sim_world`] to bootstrap a world for simulation.

use bevy_ecs::prelude::*;
use std::hash::{Hash, Hasher};

use crate::collision::collision_system;
use crate::components::{Heading, Health, MoveState, Position, SimId, Velocity};
use crate::lifecycle::{cleanup_dead, init_lifecycle};
use crate::movement::movement_system;
use crate::pathfinding::TerrainGrid;
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2};

/// Run one frame of the simulation in the correct system order.
///
/// 1. Rebuild [`SpatialGrid`] from all [`Position`] components.
/// 2. Run [`movement_system`].
/// 3. Run [`collision_system`].
/// 4. Run [`cleanup_dead`].
pub fn sim_tick(world: &mut World) {
    // 1. Rebuild spatial grid
    {
        let entities: Vec<(Entity, SimVec2)> = world
            .query::<(Entity, &Position)>()
            .iter(world)
            .map(|(e, p)| (e, SimVec2::new(p.pos.x, p.pos.z)))
            .collect();

        let mut grid = world.resource_mut::<SpatialGrid>();
        grid.clear();
        for (entity, pos) in entities {
            grid.insert(entity, pos);
        }
    }

    // 2. Movement
    movement_system(world);

    // 3. Collision
    collision_system(world);

    // 4. Cleanup dead entities
    cleanup_dead(world);
}

/// Compute a deterministic hash of all sim-relevant state.
///
/// Queries all entities with [`SimId`], sorted by `SimId.id`, and hashes
/// their core components: SimId, Position, Velocity, Heading, Health, MoveState.
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
    }

    hasher.finish()
}

/// Initialize a world for simulation.
///
/// Inserts lifecycle resources, a [`SpatialGrid`], and a [`TerrainGrid`].
pub fn init_sim_world(world: &mut World) {
    init_lifecycle(world);
    world.insert_resource(SpatialGrid::new(SimFloat::from_int(16), 64, 64));
    world.insert_resource(TerrainGrid::new(64, 64, SimFloat::ONE));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position,
        UnitType, Velocity,
    };
    use crate::lifecycle::spawn_unit;
    use crate::{SimFloat, SimVec3};

    /// Deterministic seeded RNG (linear congruential generator).
    struct SeededRng(u64);

    impl SeededRng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }

        fn next_simfloat(&mut self, min: i32, max: i32) -> SimFloat {
            let range = (max - min) as u64;
            let val = min as i64 + (self.next() % range) as i64;
            SimFloat::from_int(val as i32)
        }
    }

    /// Spawn a fully-equipped unit with movement components at a given position.
    fn spawn_full_unit(world: &mut World, x: SimFloat, z: SimFloat, team: u8) -> Entity {
        let entity = spawn_unit(
            world,
            Position {
                pos: SimVec3::new(x, SimFloat::ZERO, z),
            },
            UnitType { id: 1 },
            Allegiance { team },
            Health {
                current: SimFloat::from_int(100),
                max: SimFloat::from_int(100),
            },
        );
        world.entity_mut(entity).insert((
            Velocity { vel: SimVec3::ZERO },
            Heading {
                angle: SimFloat::ZERO,
            },
            MoveState::Idle,
            MovementParams {
                max_speed: SimFloat::from_ratio(1, 2),
                acceleration: SimFloat::from_ratio(1, 4),
                turn_rate: SimFloat::from_ratio(1, 4),
            },
            CollisionRadius {
                radius: SimFloat::ONE,
            },
        ));
        entity
    }

    // ---- Test: empty world tick ----

    #[test]
    fn test_empty_world_tick() {
        let mut world = World::new();
        init_sim_world(&mut world);
        // Ticking an empty world should not crash.
        for _ in 0..10 {
            sim_tick(&mut world);
        }
    }

    // ---- Test: single unit movement determinism ----

    #[test]
    fn test_single_unit_movement_determinism() {
        fn run() -> Vec<u64> {
            let mut world = World::new();
            init_sim_world(&mut world);

            let entity = spawn_full_unit(
                &mut world,
                SimFloat::from_int(10),
                SimFloat::from_int(10),
                1,
            );
            *world.get_mut::<MoveState>(entity).unwrap() = MoveState::MovingTo(SimVec3::new(
                SimFloat::from_int(30),
                SimFloat::ZERO,
                SimFloat::from_int(30),
            ));

            let mut checksums = Vec::new();
            for _ in 0..100 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }
            checksums
        }

        let a = run();
        let b = run();
        assert_eq!(a, b, "single unit movement must be deterministic");
    }

    // ---- Test: dual world determinism with command injection ----

    #[test]
    fn test_dual_world_determinism() {
        fn run_world(seed: u64) -> Vec<u64> {
            let mut world = World::new();
            init_sim_world(&mut world);
            let mut rng = SeededRng(seed);

            // Spawn 20 units at deterministic positions.
            let mut entities = Vec::new();
            for i in 0..20u8 {
                let x = SimFloat::from_int(10 + (i as i32) * 2);
                let z = SimFloat::from_int(10 + (i as i32) * 3);
                let entity = spawn_full_unit(&mut world, x, z, i % 4);
                entities.push(entity);
            }

            // Set initial random move targets.
            for &entity in &entities {
                let tx = rng.next_simfloat(5, 60);
                let tz = rng.next_simfloat(5, 60);
                *world.get_mut::<MoveState>(entity).unwrap() =
                    MoveState::MovingTo(SimVec3::new(tx, SimFloat::ZERO, tz));
            }

            let mut checksums = Vec::new();

            // Tick 200 frames.
            for frame in 0..200 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));

                // At frame 100, change some move targets.
                if frame == 99 {
                    for &entity in &entities[0..10] {
                        let tx = rng.next_simfloat(5, 60);
                        let tz = rng.next_simfloat(5, 60);
                        *world.get_mut::<MoveState>(entity).unwrap() =
                            MoveState::MovingTo(SimVec3::new(tx, SimFloat::ZERO, tz));
                    }
                }
            }

            // Continue for 200 more frames.
            for _ in 0..200 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }

            checksums
        }

        let seed = 42;
        let trace_a = run_world(seed);
        let trace_b = run_world(seed);

        assert_eq!(trace_a.len(), trace_b.len());
        for (i, (a, b)) in trace_a.iter().zip(trace_b.iter()).enumerate() {
            assert_eq!(a, b, "desync at frame {i}: {a} != {b}");
        }
    }

    // ---- Test: many units stress ----

    #[test]
    fn test_many_units_stress() {
        fn run() -> Vec<u64> {
            let mut world = World::new();
            init_sim_world(&mut world);
            let mut rng = SeededRng(12345);

            // Spawn 100 units.
            let mut entities = Vec::new();
            for i in 0..100u8 {
                let x = rng.next_simfloat(2, 60);
                let z = rng.next_simfloat(2, 60);
                let entity = spawn_full_unit(&mut world, x, z, i % 4);
                entities.push(entity);
            }

            // Set random move targets.
            for &entity in &entities {
                let tx = rng.next_simfloat(2, 60);
                let tz = rng.next_simfloat(2, 60);
                *world.get_mut::<MoveState>(entity).unwrap() =
                    MoveState::MovingTo(SimVec3::new(tx, SimFloat::ZERO, tz));
            }

            let mut checksums = Vec::new();
            for _ in 0..500 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }
            checksums
        }

        let a = run();
        let b = run();
        assert_eq!(a, b, "stress test must be deterministic");
    }
}
