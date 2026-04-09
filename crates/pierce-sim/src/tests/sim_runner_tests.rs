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
            current: 100,
            max: 100,
        },
    );
    world.entity_mut(entity).insert((
        Velocity { vel: SimVec3::ZERO },
        Heading {
            angle: pierce_math::Angle::ZERO,
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
            current: hp,
            max: hp,
        },
    );
    world.entity_mut(entity).insert((
        Velocity { vel: SimVec3::ZERO },
        Heading {
            angle: pierce_math::Angle::ZERO,
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
            is_paralyzer: false, ..Default::default()
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
            is_paralyzer: false, ..Default::default()
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
                    h.current = 0;
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
                is_paralyzer: false, ..Default::default()
            },
            WeaponDef {
                damage: SimFloat::from_int(20),
                damage_type: DamageType::Explosive,
                range: SimFloat::from_int(35),
                reload_time: 10,
                projectile_speed: SimFloat::from_int(2),
                area_of_effect: SimFloat::from_int(5),
                is_paralyzer: false, ..Default::default()
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
use crate::footprint::mark_building_footprint;

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
                current: 100,
                max: 100,
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
