//! Load test binary for flamegraph profiling.
//!
//! Spawns many armed units across two teams and runs the full game loop.
//! Usage:
//!   cargo build --release -p bar-game-lib --bench loadtest
//!   cargo flamegraph --bench loadtest -p bar-game-lib -- [UNITS] [FRAMES]
//!
//! Defaults: 500 units per team (1000 total), 600 frames.

use std::path::Path;
use std::time::Instant;

use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec3};
use pierce_sim::combat_data::{ArmorClass, DamageType, WeaponDef, WeaponInstance, WeaponSet};
use pierce_sim::construction::construction_system;
use pierce_sim::economy::EconomyState;
use pierce_sim::sim_runner::sim_tick;
use pierce_sim::targeting::WeaponRegistry;
use pierce_sim::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, SightRange,
    Target, UnitType, Velocity,
};

use bar_game_lib::building;
use bar_game_lib::GameState;

fn register_weapon(world: &mut World) -> u32 {
    let mut registry = world.resource_mut::<WeaponRegistry>();
    let id = registry.defs.len() as u32;
    registry.defs.push(WeaponDef {
        damage: SimFloat::from_int(25),
        damage_type: DamageType::Normal,
        range: SimFloat::from_int(250),
        reload_time: 15,
        projectile_speed: SimFloat::from_int(10),
        area_of_effect: SimFloat::ZERO,
        is_paralyzer: false,
        ..Default::default()
    });
    id
}

fn spawn_unit(world: &mut World, x: i32, z: i32, team: u8, weapon_id: u32) {
    let entity = pierce_sim::lifecycle::spawn_unit(
        world,
        Position {
            pos: SimVec3::new(SimFloat::from_int(x), SimFloat::ZERO, SimFloat::from_int(z)),
        },
        UnitType { id: 1 },
        Allegiance { team },
        Health {
            current: 500,
            max: 500,
        },
    );
    world.entity_mut(entity).insert((
        MoveState::Idle,
        MovementParams {
            max_speed: SimFloat::from_int(2),
            acceleration: SimFloat::ONE,
            turn_rate: SimFloat::ONE,
        },
        CollisionRadius {
            radius: SimFloat::from_int(8),
        },
        Heading {
            angle: pierce_math::Angle::ZERO,
        },
        Velocity { vel: SimVec3::ZERO },
        ArmorClass::Light,
        Target { entity: None },
        WeaponSet {
            weapons: vec![WeaponInstance {
                def_id: weapon_id,
                reload_remaining: 0,
            }],
        },
        SightRange {
            range: SimFloat::from_int(300),
        },
        pierce_sim::commands::CommandQueue::default(),
    ));
}

fn fund_teams(world: &mut World) {
    let mut eco = world.resource_mut::<EconomyState>();
    for team in [0u8, 1] {
        if let Some(res) = eco.teams.get_mut(&team) {
            res.metal = SimFloat::from_int(999999);
            res.energy = SimFloat::from_int(999999);
            res.metal_storage = SimFloat::from_int(999999);
            res.energy_storage = SimFloat::from_int(999999);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let units_per_team: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500);
    let frames: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(600);

    eprintln!(
        "Load test: {} units/team ({} total), {} frames",
        units_per_team,
        units_per_team * 2,
        frames
    );

    // Setup
    let bar_units = Path::new("nonexistent/units");
    let map_manifest = Path::new("assets/maps/small_duel/manifest.ron");
    let mut game = GameState::with_options(
        bar_units,
        map_manifest,
        bar_game_lib::InitOptions { fog_of_war: false },
    );
    fund_teams(&mut game.world);

    // Wire GPU compute backends when feature is enabled.
    #[cfg(feature = "gpu-compute")]
    {
        let (device, queue) = pierce_compute::create_headless_device();
        let fog = pierce_compute::GpuComputeManager::new(device.clone(), queue.clone());
        let targeting = pierce_compute::GpuTargetingCompute::new(device.clone(), queue.clone());
        game.world.insert_resource(pierce_sim::compute::ComputeBackends {
            fog: Box::new(fog),
            targeting: Box::new(targeting),
        });

        // Batch math backend (GPU for distance_sq, CPU for rest).
        let (device2, queue2) = pierce_compute::create_headless_device();
        let batch = pierce_compute::GpuBatchMath::new(device2, queue2);
        game.world.insert_resource(pierce_sim::compute::BatchMathBackend {
            ops: Box::new(batch),
        });
        eprintln!("GPU compute + batch math backends enabled");
    }

    // CPU batch math when gpu-compute is disabled (no compute-backends feature).
    #[cfg(not(feature = "gpu-compute"))]
    {
        eprintln!("Running without compute backends (inline CPU path)");
    }

    let weapon_id = register_weapon(&mut game.world);

    // Spawn team 0 on the left, team 1 on the right, marching toward each other
    let cols = (units_per_team as f64).sqrt().ceil() as i32;
    let spacing = 20;

    for i in 0..units_per_team {
        let row = i as i32 / cols;
        let col = i as i32 % cols;
        // Team 0: left side
        spawn_unit(
            &mut game.world,
            100 + col * spacing,
            100 + row * spacing,
            0,
            weapon_id,
        );
        // Team 1: right side
        spawn_unit(
            &mut game.world,
            600 + col * spacing,
            100 + row * spacing,
            1,
            weapon_id,
        );
    }

    // Issue attack-move commands toward the center
    {
        use pierce_sim::commands::CommandQueue;
        use pierce_sim::Command;

        let mut to_cmd: Vec<(bevy_ecs::entity::Entity, u8)> = game
            .world
            .query::<(bevy_ecs::entity::Entity, &Allegiance, &CommandQueue)>()
            .iter(&game.world)
            .map(|(e, a, _)| (e, a.team))
            .collect();
        to_cmd.sort_by_key(|(e, _)| e.index());

        for (entity, team) in to_cmd {
            let target_x = if team == 0 { 500 } else { 200 };
            if let Some(mut cq) = game.world.get_mut::<CommandQueue>(entity) {
                cq.replace(Command::Move(SimVec3::new(
                    SimFloat::from_int(target_x),
                    SimFloat::ZERO,
                    SimFloat::from_int(400),
                )));
            }
        }
    }

    eprintln!("Setup complete. Running simulation...");

    let start = Instant::now();
    for frame in 0..frames {
        construction_system(&mut game.world);
        sim_tick(&mut game.world);
        building::equip_factory_spawned_units(&mut game.world, &game.weapon_def_ids);
        building::finalize_completed_buildings(&mut game.world);
        game.frame_count = frame + 1;

        if frame != 0 && frame % 100 == 0 {
            let alive: usize = game
                .world
                .query_filtered::<&Allegiance, bevy_ecs::query::Without<pierce_sim::Dead>>()
                .iter(&game.world)
                .count();
            let elapsed = start.elapsed();
            let fps = (frame + 1) as f64 / elapsed.as_secs_f64();
            eprintln!(
                "  frame {}/{}: {} alive, {:.1} fps",
                frame + 1,
                frames,
                alive,
                fps
            );
        }
    }

    let elapsed = start.elapsed();
    let avg_fps = frames as f64 / elapsed.as_secs_f64();
    eprintln!(
        "Done: {} frames in {:.2}s ({:.1} avg fps)",
        frames,
        elapsed.as_secs_f64(),
        avg_fps
    );
}
