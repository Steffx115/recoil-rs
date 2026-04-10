//! Headless network client for profiling and testing.
//!
//! Connects to a server, sets up a full game (units, weapons, economy),
//! receives FrameAdvance messages, and runs sim_tick locally.
//! Exits after a configurable number of frames.
//!
//! Usage: `bar-headless-client [HOST:PORT] [UNITS_PER_TEAM] [FRAMES]`
//! Defaults: 127.0.0.1:7878, 500 units/team, 600 frames

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use bevy_ecs::world::World;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use pierce_math::{SimFloat, SimVec3};
use pierce_net::protocol::NetMessage;
use pierce_net::{decode, encode_framed};
use pierce_sim::combat_data::{ArmorClass, DamageType, WeaponDef, WeaponInstance, WeaponSet};
use pierce_sim::economy::EconomyState;
use pierce_sim::targeting::WeaponRegistry;
use pierce_sim::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, SightRange,
    Target, UnitType, Velocity,
};

use bar_game_lib::net::NetGame;
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

fn setup_game(units_per_team: usize) -> NetGame {
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

        let (device2, queue2) = pierce_compute::create_headless_device();
        let batch = pierce_compute::GpuBatchMath::new(device2, queue2);
        game.world.insert_resource(pierce_sim::compute::BatchMathBackend {
            ops: Box::new(batch),
        });
        eprintln!("GPU compute + batch math backends enabled");
    }

    #[cfg(not(feature = "gpu-compute"))]
    {
        eprintln!("Running without compute backends (inline CPU path)");
    }

    let weapon_id = register_weapon(&mut game.world);

    // Spawn team 0 on the left, team 1 on the right.
    let cols = (units_per_team as f64).sqrt().ceil() as i32;
    let spacing = 20;

    for i in 0..units_per_team {
        let row = i as i32 / cols;
        let col = i as i32 % cols;
        spawn_unit(&mut game.world, 100 + col * spacing, 100 + row * spacing, 0, weapon_id);
        spawn_unit(&mut game.world, 600 + col * spacing, 100 + row * spacing, 1, weapon_id);
    }

    // Issue attack-move commands toward the center.
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

    NetGame::new(game)
}

#[tokio::main]
async fn main() -> Result<()> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7878".to_string());

    let units_per_team: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let max_frames: u64 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);

    eprintln!(
        "Headless client: {} units/team ({} total), {} frames",
        units_per_team,
        units_per_team * 2,
        max_frames,
    );
    eprintln!("Connecting to {addr}...");

    let mut stream = TcpStream::connect(&addr).await?;

    // Wait for Hello.
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    match decode(&buf)? {
        NetMessage::Hello { player_id, game_id } => {
            eprintln!("Joined game {game_id} as player {player_id}");
        }
        _ => {
            eprintln!("ERROR: expected Hello from server");
            return Ok(());
        }
    }

    // Set up the full game with units.
    let mut net_game = setup_game(units_per_team);
    eprintln!("Setup complete. Waiting for ticks...");

    let start = Instant::now();
    let mut read_buf = vec![0u8; 64 * 1024];

    loop {
        // Read length-prefixed frame from server.
        if stream.read_exact(&mut len_buf).await.is_err() {
            eprintln!("Server disconnected");
            break;
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > read_buf.len() {
            read_buf.resize(len, 0);
        }
        if stream.read_exact(&mut read_buf[..len]).await.is_err() {
            eprintln!("Server disconnected");
            break;
        }

        match decode(&read_buf[..len]) {
            Ok(msg @ NetMessage::FrameAdvance { .. }) => {
                net_game.receive(&msg);
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("Bad message from server: {e}");
            }
        }

        // Process buffered frames.
        let stats = net_game.process_ticks();

        if stats.ticks_run > 0 && stats.total_frames % 100 == 0 {
            let elapsed = start.elapsed();
            let fps = stats.total_frames as f64 / elapsed.as_secs_f64();
            eprintln!(
                "  frame {}/{}: {} alive, {:.1} fps, {} behind ({:?})",
                stats.total_frames, max_frames, stats.alive_count, fps,
                stats.frames_behind, stats.adapt_level,
            );
        }

        if net_game.game.frame_count >= max_frames {
            let checksum = net_game.checksum();
            let elapsed = start.elapsed();
            let avg_fps = max_frames as f64 / elapsed.as_secs_f64();
            eprintln!(
                "Done: {} frames in {:.2}s ({:.1} avg fps), checksum: {:#018x}",
                max_frames, elapsed.as_secs_f64(), avg_fps, checksum,
            );

            let msg = encode_framed(&NetMessage::Checksum {
                frame: max_frames,
                hash: checksum,
            });
            let _ = stream.write_all(&msg).await;

            return Ok(());
        }
    }

    eprintln!("Exited after {} frames", net_game.game.frame_count);
    Ok(())
}
