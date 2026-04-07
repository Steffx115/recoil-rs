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
use crate::pathfinding::{footprint_cleanup_system, TerrainGrid};
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
mod tests {
    use super::*;
    use crate::combat_data::{ArmorClass, DamageType, WeaponDef, WeaponInstance, WeaponSet};
    use crate::commands::CommandQueue;
    use crate::components::{
        Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, Target,
        UnitType, Velocity,
    };
    use crate::economy::{init_economy, ResourceConsumer, ResourceProducer};
    use crate::lifecycle::{spawn_unit, SimIdCounter};
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

    /// Spawn a unit with all combat components (weapons, armor, targeting).
    fn spawn_armed_unit(
        world: &mut World,
        x: SimFloat,
        z: SimFloat,
        team: u8,
        weapon_def_id: u32,
        hp: i32,
    ) -> Entity {
        let entity = spawn_unit(
            world,
            Position {
                pos: SimVec3::new(x, SimFloat::ZERO, z),
            },
            UnitType { id: 1 },
            Allegiance { team },
            Health {
                current: SimFloat::from_int(hp),
                max: SimFloat::from_int(hp),
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
            WeaponSet {
                weapons: vec![WeaponInstance {
                    def_id: weapon_def_id,
                    reload_remaining: 0,
                }],
            },
            ArmorClass::Light,
            Target { entity: None },
            CommandQueue::default(),
        ));
        entity
    }

    /// Initialize a world with combat-ready weapon registry.
    fn init_combat_world(weapon_defs: Vec<WeaponDef>) -> World {
        let mut world = World::new();
        world.insert_resource(WeaponRegistry { defs: weapon_defs });
        init_sim_world(&mut world);
        world
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

    // ---- Test: combat determinism (targeting, reload, projectile, damage, death) ----

    #[test]
    fn test_combat_determinism() {
        fn run() -> Vec<u64> {
            // Weapon: 10 damage, Normal, range 30, reload 5, projectile speed 3.
            let weapon_def = WeaponDef {
                damage: SimFloat::from_int(10),
                damage_type: DamageType::Normal,
                range: SimFloat::from_int(30),
                reload_time: 5,
                projectile_speed: SimFloat::from_int(3),
                area_of_effect: SimFloat::ZERO,
                is_paralyzer: false,
            };

            let mut world = init_combat_world(vec![weapon_def]);
            let mut rng = SeededRng(99);

            // Spawn 50 units per team (100 total), spread across the map.
            for _ in 0..50 {
                let x = rng.next_simfloat(5, 55);
                let z = rng.next_simfloat(5, 55);
                spawn_armed_unit(&mut world, x, z, 1, 0, 100);
            }
            for _ in 0..50 {
                let x = rng.next_simfloat(5, 55);
                let z = rng.next_simfloat(5, 55);
                spawn_armed_unit(&mut world, x, z, 2, 0, 100);
            }

            let mut checksums = Vec::new();
            for _ in 0..1000 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }
            checksums
        }

        let a = run();
        let b = run();

        assert_eq!(a.len(), b.len());
        for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(ca, cb, "combat desync at frame {i}");
        }
    }

    // ---- Test: economy determinism ----

    #[test]
    fn test_economy_determinism() {
        fn run() -> Vec<u64> {
            let mut world = World::new();
            init_sim_world(&mut world);
            init_economy(&mut world, &[1, 2]);

            // Team 1: producer + consumer
            world.spawn((
                Allegiance { team: 1 },
                ResourceProducer {
                    metal_per_tick: SimFloat::from_int(5),
                    energy_per_tick: SimFloat::from_int(3),
                },
            ));
            world.spawn((
                Allegiance { team: 1 },
                ResourceConsumer {
                    metal_per_tick: SimFloat::from_int(8),
                    energy_per_tick: SimFloat::from_int(2),
                },
            ));

            // Team 2: two producers, one consumer
            world.spawn((
                Allegiance { team: 2 },
                ResourceProducer {
                    metal_per_tick: SimFloat::from_int(10),
                    energy_per_tick: SimFloat::from_int(10),
                },
            ));
            world.spawn((
                Allegiance { team: 2 },
                ResourceProducer {
                    metal_per_tick: SimFloat::from_int(2),
                    energy_per_tick: SimFloat::from_int(1),
                },
            ));
            world.spawn((
                Allegiance { team: 2 },
                ResourceConsumer {
                    metal_per_tick: SimFloat::from_int(15),
                    energy_per_tick: SimFloat::from_int(5),
                },
            ));

            let mut checksums = Vec::new();
            for _ in 0..500 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }
            checksums
        }

        let a = run();
        let b = run();
        assert_eq!(a, b, "economy must be deterministic");
    }

    // ---- Test: rapid spawn/despawn determinism ----

    #[test]
    fn test_rapid_spawn_despawn() {
        fn run() -> (Vec<u64>, u64) {
            let weapon_def = WeaponDef {
                damage: SimFloat::from_int(10),
                damage_type: DamageType::Normal,
                range: SimFloat::from_int(20),
                reload_time: 3,
                projectile_speed: SimFloat::ZERO, // beam = instant
                area_of_effect: SimFloat::ZERO,
                is_paralyzer: false,
            };

            let mut world = init_combat_world(vec![weapon_def]);
            let mut rng = SeededRng(777);
            let mut checksums = Vec::new();

            for _cycle in 0..200 {
                // Spawn 50 units.
                let mut batch = Vec::new();
                for _ in 0..50 {
                    let x = rng.next_simfloat(2, 60);
                    let z = rng.next_simfloat(2, 60);
                    let team = if rng.next().is_multiple_of(2) { 1 } else { 2 };
                    let e = spawn_armed_unit(&mut world, x, z, team, 0, 50);
                    batch.push(e);
                }

                // Kill 25 of them (set health to zero, damage_system marks dead).
                for &e in &batch[..25] {
                    if let Some(mut h) = world.get_mut::<Health>(e) {
                        h.current = SimFloat::ZERO;
                    }
                }

                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }

            let counter = world.resource::<SimIdCounter>().next_id;
            (checksums, counter)
        }

        let (checksums_a, counter_a) = run();
        let (checksums_b, counter_b) = run();

        assert_eq!(counter_a, counter_b, "SimId counter must match");
        assert_eq!(checksums_a.len(), checksums_b.len());
        for (i, (a, b)) in checksums_a.iter().zip(checksums_b.iter()).enumerate() {
            assert_eq!(a, b, "spawn/despawn desync at cycle {i}");
        }
    }

    // ---- Test: pathfinding determinism with obstacles ----

    #[test]
    fn test_pathfinding_determinism() {
        fn run() -> Vec<u64> {
            let mut world = World::new();
            init_sim_world(&mut world);

            // Create obstacles on the terrain grid (set cost to zero = impassable).
            {
                let mut grid = world.resource_mut::<TerrainGrid>();
                // Wall of obstacles from (30,0) to (30,20).
                for y in 0..20 {
                    grid.set(30, y, SimFloat::ZERO);
                }
            }

            let mut rng = SeededRng(555);

            // Spawn 20 units on the left side, all moving to the right side.
            let mut entities = Vec::new();
            for _ in 0..20 {
                let x = rng.next_simfloat(5, 25);
                let z = rng.next_simfloat(5, 55);
                let entity = spawn_full_unit(&mut world, x, z, 1);
                entities.push(entity);
            }

            // Set all units to move to the same goal on the right side.
            let goal = SimVec3::new(
                SimFloat::from_int(50),
                SimFloat::ZERO,
                SimFloat::from_int(30),
            );
            for &entity in &entities {
                *world.get_mut::<MoveState>(entity).unwrap() = MoveState::MovingTo(goal);
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

        assert_eq!(a.len(), b.len());
        for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(ca, cb, "pathfinding desync at frame {i}");
        }
    }

    // ---- Test: full combat stress 10000 frames (slow, ignored by default) ----

    #[test]
    #[ignore]
    fn test_full_combat_stress_10000_frames() {
        fn run() -> Vec<u64> {
            // Two weapon types for variety.
            let weapon_defs = vec![
                WeaponDef {
                    damage: SimFloat::from_int(8),
                    damage_type: DamageType::Normal,
                    range: SimFloat::from_int(25),
                    reload_time: 4,
                    projectile_speed: SimFloat::from_int(4),
                    area_of_effect: SimFloat::ZERO,
                    is_paralyzer: false,
                },
                WeaponDef {
                    damage: SimFloat::from_int(20),
                    damage_type: DamageType::Explosive,
                    range: SimFloat::from_int(35),
                    reload_time: 10,
                    projectile_speed: SimFloat::from_int(2),
                    area_of_effect: SimFloat::from_int(5),
                    is_paralyzer: false,
                },
            ];

            let mut world = init_combat_world(weapon_defs);
            init_economy(&mut world, &[1, 2, 3, 4]);

            let mut rng = SeededRng(42424242);

            // Spawn 200 units across 4 teams.
            for i in 0..200u32 {
                let x = rng.next_simfloat(2, 60);
                let z = rng.next_simfloat(2, 60);
                let team = (i % 4) as u8 + 1;
                let weapon_id = (rng.next() % 2) as u32;
                spawn_armed_unit(&mut world, x, z, team, weapon_id, 150);
            }

            // Add economy producers and consumers for each team.
            for team in 1..=4u8 {
                world.spawn((
                    Allegiance { team },
                    ResourceProducer {
                        metal_per_tick: SimFloat::from_int(5),
                        energy_per_tick: SimFloat::from_int(5),
                    },
                ));
                world.spawn((
                    Allegiance { team },
                    ResourceConsumer {
                        metal_per_tick: SimFloat::from_int(3),
                        energy_per_tick: SimFloat::from_int(2),
                    },
                ));
            }

            // Sample checksums every 100 frames to keep memory low.
            let mut checksums = Vec::new();
            for frame in 0..10000 {
                sim_tick(&mut world);
                if frame % 100 == 0 {
                    checksums.push(world_checksum(&mut world));
                }
            }
            checksums
        }

        let a = run();
        let b = run();

        assert_eq!(a.len(), b.len());
        for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(
                ca,
                cb,
                "full stress desync at sample {i} (frame {})",
                i * 100
            );
        }
    }

    // ==================================================================
    // Building footprint + pathfinding integration tests (RR-102)
    // ==================================================================

    use crate::commands::Command;
    use crate::components::Dead;
    use crate::pathfinding::mark_building_footprint;

    // ---- Test: unit paths around a building in full sim tick ----

    #[test]
    fn test_unit_paths_around_building() {
        let mut world = World::new();
        init_sim_world(&mut world);

        // Place a "building" blocking the direct path at column 15.
        let building_pos = SimVec2::new(SimFloat::from_int(15), SimFloat::from_int(10));
        let fp = {
            let mut grid = world.resource_mut::<TerrainGrid>();
            mark_building_footprint(&mut grid, building_pos, SimFloat::from_int(4))
        };

        // Spawn the building entity with footprint + collision.
        let _building = world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(15),
                        SimFloat::ZERO,
                        SimFloat::from_int(10),
                    ),
                },
                CollisionRadius {
                    radius: SimFloat::from_int(4),
                },
                fp,
            ))
            .id();

        // Spawn a unit at (5, 10) that needs to reach (25, 10).
        let unit = spawn_full_unit(&mut world, SimFloat::from_int(5), SimFloat::from_int(10), 1);
        world.entity_mut(unit).insert(CommandQueue::default());
        world
            .get_mut::<CommandQueue>(unit)
            .unwrap()
            .push(Command::Move(SimVec3::new(
                SimFloat::from_int(25),
                SimFloat::ZERO,
                SimFloat::from_int(10),
            )));

        // Run several ticks.
        for _ in 0..200 {
            sim_tick(&mut world);
        }

        // Unit should have moved past column 15 (the building).
        let unit_pos = world.get::<Position>(unit).unwrap().pos;
        assert!(
            unit_pos.x > SimFloat::from_int(20),
            "unit should have pathed around building, x = {}",
            unit_pos.x.to_f64()
        );
    }

    // ---- Test: building destruction restores terrain ----

    #[test]
    fn test_building_death_restores_terrain() {
        let mut world = World::new();
        init_sim_world(&mut world);

        let building_pos = SimVec2::new(SimFloat::from_int(10), SimFloat::from_int(10));
        let fp = {
            let mut grid = world.resource_mut::<TerrainGrid>();
            mark_building_footprint(&mut grid, building_pos, SimFloat::from_int(2))
        };
        let fp_cells = fp.cells.clone();

        let building = world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(10),
                        SimFloat::ZERO,
                        SimFloat::from_int(10),
                    ),
                },
                Health {
                    current: SimFloat::from_int(100),
                    max: SimFloat::from_int(100),
                },
                fp,
            ))
            .id();

        // Verify cells are blocked.
        {
            let grid = world.resource::<TerrainGrid>();
            for &(x, y) in &fp_cells {
                assert!(!grid.is_passable(x, y));
            }
        }

        // Kill the building.
        world.entity_mut(building).insert(Dead);
        sim_tick(&mut world);

        // Building should be despawned.
        assert!(world.get_entity(building).is_err());

        // Terrain should be restored.
        {
            let grid = world.resource::<TerrainGrid>();
            for &(x, y) in &fp_cells {
                assert!(
                    grid.is_passable(x, y),
                    "cell ({x},{y}) should be passable after building death"
                );
            }
        }
    }

    // ---- Test: pathfinding determinism with buildings ----

    #[test]
    fn test_pathfinding_with_buildings_determinism() {
        fn run() -> Vec<u64> {
            let mut world = World::new();
            init_sim_world(&mut world);

            // Place several buildings to create a maze-like layout.
            let buildings = [
                (15, 5, 3),
                (15, 15, 3),
                (15, 25, 3),
                (30, 10, 4),
                (30, 20, 4),
            ];
            for &(bx, bz, r) in &buildings {
                let bp = SimVec2::new(SimFloat::from_int(bx), SimFloat::from_int(bz));
                let fp = {
                    let mut grid = world.resource_mut::<TerrainGrid>();
                    mark_building_footprint(&mut grid, bp, SimFloat::from_int(r))
                };
                world.spawn((
                    Position {
                        pos: SimVec3::new(
                            SimFloat::from_int(bx),
                            SimFloat::ZERO,
                            SimFloat::from_int(bz),
                        ),
                    },
                    CollisionRadius {
                        radius: SimFloat::from_int(r),
                    },
                    fp,
                ));
            }

            // Spawn 10 units that path through the building maze.
            let mut rng = SeededRng(42);
            for _ in 0..10 {
                let x = rng.next_simfloat(2, 10);
                let z = rng.next_simfloat(2, 30);
                let entity = spawn_full_unit(&mut world, x, z, 1);
                world.entity_mut(entity).insert(CommandQueue::default());
                let tx = rng.next_simfloat(40, 60);
                let tz = rng.next_simfloat(2, 30);
                world
                    .get_mut::<CommandQueue>(entity)
                    .unwrap()
                    .push(Command::Move(SimVec3::new(tx, SimFloat::ZERO, tz)));
            }

            let mut checksums = Vec::new();
            for _ in 0..300 {
                sim_tick(&mut world);
                checksums.push(world_checksum(&mut world));
            }
            checksums
        }

        let a = run();
        let b = run();

        assert_eq!(a.len(), b.len());
        for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(ca, cb, "building pathfinding desync at frame {i}");
        }
    }
}
