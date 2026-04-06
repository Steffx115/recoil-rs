//! Criterion benchmarks for recoil-sim hot paths.
//!
//! Run with: `cargo bench -p recoil-sim`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use bevy_ecs::prelude::*;
use recoil_sim::components::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, UnitType,
    Velocity,
};
use recoil_sim::lifecycle::spawn_unit;
use recoil_sim::sim_runner::{init_sim_world, sim_tick};
use recoil_sim::spatial::SpatialGrid;
use recoil_sim::{SimFloat, SimVec2, SimVec3};

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

/// Spawn a fully-equipped unit with movement and collision components.
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

/// Spawn `count` units at deterministic random positions, optionally with move targets.
fn setup_world_with_units(count: usize, moving: bool) -> World {
    let mut world = World::new();
    init_sim_world(&mut world);
    let mut rng = SeededRng(42);

    let mut entities = Vec::with_capacity(count);
    for i in 0..count {
        let x = rng.next_simfloat(2, 200);
        let z = rng.next_simfloat(2, 200);
        let entity = spawn_full_unit(&mut world, x, z, (i % 4) as u8);
        entities.push(entity);
    }

    if moving {
        for &entity in &entities {
            let tx = rng.next_simfloat(2, 200);
            let tz = rng.next_simfloat(2, 200);
            *world.get_mut::<MoveState>(entity).unwrap() =
                MoveState::MovingTo(SimVec3::new(tx, SimFloat::ZERO, tz));
        }
    }

    // Rebuild grid once so it's populated for systems that read it.
    rebuild_grid(&mut world);

    world
}

/// Rebuild the spatial grid from current positions.
fn rebuild_grid(world: &mut World) {
    let entities: Vec<(Entity, SimVec2)> = world
        .query_filtered::<(Entity, &Position), Without<recoil_sim::Dead>>()
        .iter(world)
        .map(|(e, p)| (e, SimVec2::new(p.pos.x, p.pos.z)))
        .collect();

    let mut grid = world.resource_mut::<SpatialGrid>();
    grid.clear();
    for (entity, pos) in entities {
        grid.insert(entity, pos);
    }
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

fn bench_spatial_grid_rebuild(c: &mut Criterion) {
    let mut world = setup_world_with_units(2000, false);

    c.bench_function("spatial_grid_rebuild_2000", |b| {
        b.iter(|| {
            rebuild_grid(black_box(&mut world));
        });
    });
}

fn bench_collision_2000(c: &mut Criterion) {
    let mut world = setup_world_with_units(2000, false);

    c.bench_function("collision_2000", |b| {
        b.iter(|| {
            // Rebuild grid each iteration to reset state.
            rebuild_grid(&mut world);
            recoil_sim::collision::collision_system(black_box(&mut world));
        });
    });
}

fn bench_movement_2000(c: &mut Criterion) {
    let mut world = setup_world_with_units(2000, true);

    c.bench_function("movement_2000", |b| {
        b.iter(|| {
            recoil_sim::movement::movement_system(black_box(&mut world));
        });
    });
}

fn bench_full_tick_2000(c: &mut Criterion) {
    let mut world = setup_world_with_units(2000, true);

    c.bench_function("full_tick_2000", |b| {
        b.iter(|| {
            sim_tick(black_box(&mut world));
        });
    });
}

criterion_group!(
    benches,
    bench_spatial_grid_rebuild,
    bench_collision_2000,
    bench_movement_2000,
    bench_full_tick_2000,
);
criterion_main!(benches);
