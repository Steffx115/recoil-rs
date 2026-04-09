//! Test helpers: game construction, funding, snapshot comparisons.

use std::path::Path;

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec3};
use pierce_sim::economy::EconomyState;
use pierce_sim::{Dead, Health, Position};

use super::GameState;

// -------------------------------------------------------------------
// Snapshot helper for negative assertions
// -------------------------------------------------------------------

/// Lightweight snapshot of game state for before/after comparison.
#[allow(dead_code)]
pub(crate) struct Snapshot {
    pub entity_count: usize,
    pub t0_count: usize,
    pub t1_count: usize,
    pub building_count: usize,
    pub cmd0_pos: Option<(f32, f32)>,
    pub cmd1_pos: Option<(f32, f32)>,
    pub cmd0_hp: Option<f32>,
    pub cmd1_hp: Option<f32>,
    pub metal_t0: f32,
    pub energy_t0: f32,
}

#[allow(dead_code)]
impl Snapshot {
    pub fn capture(game: &mut GameState) -> Self {
        let entity_count = game
            .world
            .query_filtered::<&pierce_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();
        let t0_count = game
            .world
            .query_filtered::<&pierce_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .filter(|a| a.team == 0)
            .count();
        let t1_count = game
            .world
            .query_filtered::<&pierce_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .filter(|a| a.team == 1)
            .count();
        let building_count = game
            .world
            .query_filtered::<&pierce_sim::construction::BuildSite, Without<Dead>>()
            .iter(&game.world)
            .count();
        let cmd0_pos = game
            .commander_team0
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));
        let cmd1_pos = game
            .commander_team1
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));
        let cmd0_hp = game
            .commander_team0
            .and_then(|e| game.world.get::<Health>(e))
            .map(|h| h.current as f32);
        let cmd1_hp = game
            .commander_team1
            .and_then(|e| game.world.get::<Health>(e))
            .map(|h| h.current as f32);
        let (metal_t0, energy_t0) = {
            let eco = game.world.resource::<EconomyState>();
            eco.teams
                .get(&0)
                .map(|r| (r.metal.to_f32(), r.energy.to_f32()))
                .unwrap_or((0.0, 0.0))
        };
        Self {
            entity_count,
            t0_count,
            t1_count,
            building_count,
            cmd0_pos,
            cmd1_pos,
            cmd0_hp,
            cmd1_hp,
            metal_t0,
            energy_t0,
        }
    }

    pub fn assert_entity_count_unchanged(&self, game: &mut GameState, msg: &str) {
        let now = game
            .world
            .query_filtered::<&pierce_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();
        assert_eq!(
            self.entity_count, now,
            "Entity count changed unexpectedly: {}",
            msg
        );
    }

    pub fn assert_t0_count_unchanged(&self, game: &mut GameState, msg: &str) {
        let now = game
            .world
            .query_filtered::<&pierce_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .filter(|a| a.team == 0)
            .count();
        assert_eq!(self.t0_count, now, "Team 0 count changed: {}", msg);
    }

    pub fn assert_t1_count_unchanged(&self, game: &mut GameState, msg: &str) {
        let now = game
            .world
            .query_filtered::<&pierce_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .filter(|a| a.team == 1)
            .count();
        assert_eq!(self.t1_count, now, "Team 1 count changed: {}", msg);
    }

    pub fn assert_cmd0_pos_unchanged(&self, game: &GameState, msg: &str) {
        let now = game
            .commander_team0
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));
        assert_eq!(self.cmd0_pos, now, "Cmd0 position changed: {}", msg);
    }

    pub fn assert_cmd1_pos_unchanged(&self, game: &GameState, msg: &str) {
        let now = game
            .commander_team1
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));
        assert_eq!(self.cmd1_pos, now, "Cmd1 position changed: {}", msg);
    }

    pub fn assert_no_new_buildings(&self, game: &mut GameState, msg: &str) {
        let now = game
            .world
            .query_filtered::<&pierce_sim::construction::BuildSite, Without<Dead>>()
            .iter(&game.world)
            .count();
        assert_eq!(self.building_count, now, "BuildSite count changed: {}", msg);
    }

    pub fn assert_cmd0_hp_unchanged(&self, game: &GameState, msg: &str) {
        let now = game
            .commander_team0
            .and_then(|e| game.world.get::<Health>(e))
            .map(|h| h.current as f32);
        assert_eq!(self.cmd0_hp, now, "Cmd0 HP changed: {}", msg);
    }
}

/// Helper: create a GameState with fallback defs (no BAR repo needed).
pub(crate) fn make_test_game() -> GameState {
    let bar_units = Path::new("nonexistent/units");
    let map_manifest = Path::new("assets/maps/small_duel/manifest.ron");
    GameState::new(bar_units, map_manifest)
}

/// Helper: create a GameState with fog of war disabled.
pub(crate) fn make_test_game_no_fog() -> GameState {
    let bar_units = Path::new("nonexistent/units");
    let map_manifest = Path::new("assets/maps/small_duel/manifest.ron");
    GameState::with_options(
        bar_units,
        map_manifest,
        crate::setup::InitOptions { fog_of_war: false },
    )
}

/// Give both teams abundant resources.
pub(crate) fn fund_both_teams(game: &mut GameState) {
    let mut economy = game.world.resource_mut::<EconomyState>();
    for team in [0u8, 1] {
        if let Some(res) = economy.teams.get_mut(&team) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(100000);
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }
}

/// Helper: fund a single team.
pub(crate) fn fund_team(game: &mut GameState, team: u8) {
    let mut economy = game.world.resource_mut::<EconomyState>();
    if let Some(res) = economy.teams.get_mut(&team) {
        res.metal = SimFloat::from_int(50000);
        res.energy = SimFloat::from_int(100000);
        res.metal_storage = SimFloat::from_int(100000);
        res.energy_storage = SimFloat::from_int(200000);
    }
}

/// Assert that the terrain grid is large enough to contain a building placed
/// at `(x, z)` with the given `radius`.  Catches tests silently bypassing
/// footprint validation because coordinates fall outside the grid.
pub(crate) fn assert_grid_covers(game: &GameState, x: f32, z: f32, radius: f32) {
    let grid = game
        .world
        .resource::<pierce_sim::pathfinding::TerrainGrid>();
    let max_x = (x + radius).ceil() as usize;
    let max_z = (z + radius).ceil() as usize;
    assert!(
        max_x < grid.width() && max_z < grid.height(),
        "Terrain grid {}x{} too small for placement at ({x}, {z}) with radius {radius} \
         (needs at least {}x{}). Enlarge the grid in the test setup.",
        grid.width(),
        grid.height(),
        max_x + 1,
        max_z + 1,
    );
}

/// Snapshot all alive entity positions sorted by SimId (deterministic order).
pub(crate) fn snapshot_positions(game: &mut GameState) -> Vec<(u64, SimVec3)> {
    use pierce_sim::SimId;
    let mut positions: Vec<(u64, SimVec3)> = game
        .world
        .query_filtered::<(&SimId, &Position), Without<Dead>>()
        .iter(&game.world)
        .map(|(id, p)| (id.id, p.pos))
        .collect();
    positions.sort_by_key(|(id, _)| *id);
    positions
}

pub(crate) fn run_headless_game(frames: u64) -> Vec<Vec<(u64, SimVec3)>> {
    let mut game = make_test_game();
    fund_both_teams(&mut game);

    let mut ai0 = crate::ai::AiState::new(99, 0, 1, game.commander_team0, game.commander_team1);

    let mut snapshots = Vec::new();
    for _ in 0..frames {
        game.tick();
        crate::ai::ai_tick(&mut game.world, &mut ai0, game.frame_count);
        game.frame_count += 1;
        snapshots.push(snapshot_positions(&mut game));
    }
    snapshots
}

pub(crate) fn register_test_weapon(game: &mut GameState) -> u32 {
    use pierce_sim::combat_data::{DamageType, WeaponDef};
    use pierce_sim::targeting::WeaponRegistry;

    let mut registry = game.world.resource_mut::<WeaponRegistry>();
    let id = registry.defs.len() as u32;
    registry.defs.push(WeaponDef {
        damage: SimFloat::from_int(50),
        damage_type: DamageType::Normal,
        range: SimFloat::from_int(200),
        reload_time: 10,
        ..Default::default()
    });
    id
}

pub(crate) fn spawn_armed_unit(
    game: &mut GameState,
    x: i32,
    z: i32,
    team: u8,
    weapon_def_id: u32,
    hp: i32,
) -> Entity {
    use pierce_sim::combat_data::{ArmorClass, WeaponInstance, WeaponSet};

    let entity = pierce_sim::lifecycle::spawn_unit(
        &mut game.world,
        Position {
            pos: SimVec3::new(SimFloat::from_int(x), SimFloat::ZERO, SimFloat::from_int(z)),
        },
        pierce_sim::UnitType { id: 1 },
        pierce_sim::Allegiance { team },
        Health {
            current: hp,
            max: hp,
        },
    );
    game.world.entity_mut(entity).insert((
        pierce_sim::MoveState::Idle,
        pierce_sim::MovementParams {
            max_speed: SimFloat::from_int(2),
            acceleration: SimFloat::ONE,
            turn_rate: SimFloat::ONE,
        },
        pierce_sim::CollisionRadius {
            radius: SimFloat::from_int(8),
        },
        pierce_sim::Heading {
            angle: pierce_math::Angle::ZERO,
        },
        pierce_sim::Velocity { vel: SimVec3::ZERO },
        ArmorClass::Light,
        pierce_sim::Target { entity: None },
        WeaponSet {
            weapons: vec![WeaponInstance {
                def_id: weapon_def_id,
                reload_remaining: 0,
            }],
        },
        pierce_sim::SightRange {
            range: SimFloat::from_int(300),
        },
        pierce_sim::commands::CommandQueue::default(),
    ));
    entity
}

pub(crate) fn self_has_movestate(game: &GameState, e: Entity) -> bool {
    game.world.get_entity(e).is_ok() && game.world.get::<pierce_sim::MoveState>(e).is_some()
}

/// Helper: count entities by UnitType ID.
pub(crate) fn count_by_type(game: &mut GameState, type_id: u32) -> usize {
    game.world
        .query_filtered::<&pierce_sim::UnitType, Without<Dead>>()
        .iter(&game.world)
        .filter(|ut| ut.id == type_id)
        .count()
}

/// Helper: collect all alive entity positions as sorted vec for comparison.
pub(crate) fn all_positions(game: &mut GameState) -> Vec<(u64, f32, f32)> {
    let mut out: Vec<_> = game
        .world
        .query_filtered::<(&pierce_sim::SimId, &Position), Without<Dead>>()
        .iter(&game.world)
        .map(|(sid, p)| (sid.id, p.pos.x.to_f32(), p.pos.z.to_f32()))
        .collect();
    out.sort_by_key(|(id, _, _)| *id);
    out
}

/// Helper: collect all HP values keyed by SimId.
pub(crate) fn all_health(game: &mut GameState) -> Vec<(u64, f32, f32)> {
    let mut out: Vec<_> = game
        .world
        .query_filtered::<(&pierce_sim::SimId, &Health), Without<Dead>>()
        .iter(&game.world)
        .map(|(sid, h)| (sid.id, h.current as f32, h.max as f32))
        .collect();
    out.sort_by_key(|(id, _, _)| *id);
    out
}

/// Apply a list of PlayerCommands to the ECS world by looking up SimId.
pub(crate) fn apply_player_commands(world: &mut World, commands: &[pierce_net::PlayerCommand]) {
    use pierce_sim::SimId;
    // Build a map from SimId -> Entity
    let id_to_entity: std::collections::BTreeMap<u64, Entity> = world
        .query::<(Entity, &SimId)>()
        .iter(world)
        .map(|(e, sid)| (sid.id, e))
        .collect();

    // Collect commands to apply (can't mutate world while iterating)
    let to_apply: Vec<(Entity, pierce_sim::Command)> = commands
        .iter()
        .filter_map(|pc| {
            id_to_entity
                .get(&pc.target_sim_id)
                .map(|&e| (e, pc.command.clone()))
        })
        .collect();

    for (entity, cmd) in to_apply {
        if let Some(mut cq) = world.get_mut::<pierce_sim::CommandQueue>(entity) {
            cq.replace(cmd);
        }
    }
}

/// Run a deterministic game scenario, recording commands and checksums.
/// Returns (recorded_commands_per_frame, checksums_per_frame).
pub(crate) fn run_replay_scenario(
    tick_count: u64,
) -> (Vec<Vec<pierce_net::PlayerCommand>>, Vec<u64>) {
    use pierce_sim::sim_runner::{sim_tick, world_checksum};
    use pierce_sim::{SimId, SimVec3};

    let mut game = make_test_game();
    fund_both_teams(&mut game);

    // Collect all commandable entities (those with CommandQueue and SimId)
    let commandable: Vec<(u64, Entity)> = game
        .world
        .query::<(Entity, &SimId, &pierce_sim::CommandQueue)>()
        .iter(&game.world)
        .map(|(e, sid, _)| (sid.id, e))
        .collect();

    // Deterministic seeded RNG for command generation
    let mut rng_state: u64 = 0xDEAD_BEEF_CAFE_1234;
    let mut next_rng = || -> u64 {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        rng_state
    };

    let mut all_commands: Vec<Vec<pierce_net::PlayerCommand>> = Vec::new();
    let mut all_checksums: Vec<u64> = Vec::new();

    for frame in 0..tick_count {
        let mut frame_commands = Vec::new();

        // Every 50 frames, issue move commands to some units
        if frame % 50 == 0 && !commandable.is_empty() {
            let num_cmds = (next_rng() % 5 + 1) as usize;
            for _ in 0..num_cmds.min(commandable.len()) {
                let idx = (next_rng() as usize) % commandable.len();
                let (sim_id, _) = commandable[idx];
                let tx = (next_rng() % 800) as i32 + 50;
                let tz = (next_rng() % 800) as i32 + 50;
                frame_commands.push(pierce_net::PlayerCommand {
                    target_sim_id: sim_id,
                    command: pierce_sim::Command::Move(SimVec3::new(
                        SimFloat::from_int(tx),
                        SimFloat::ZERO,
                        SimFloat::from_int(tz),
                    )),
                });
            }
        }

        // Every 100 frames, issue stop commands
        if frame % 100 == 30 && !commandable.is_empty() {
            let idx = (next_rng() as usize) % commandable.len();
            let (sim_id, _) = commandable[idx];
            frame_commands.push(pierce_net::PlayerCommand {
                target_sim_id: sim_id,
                command: pierce_sim::Command::Stop,
            });
        }

        // Every 200 frames, issue hold position
        if frame % 200 == 75 && !commandable.is_empty() {
            let idx = (next_rng() as usize) % commandable.len();
            let (sim_id, _) = commandable[idx];
            frame_commands.push(pierce_net::PlayerCommand {
                target_sim_id: sim_id,
                command: pierce_sim::Command::HoldPosition,
            });
        }

        // Apply commands
        apply_player_commands(&mut game.world, &frame_commands);

        // Run construction + sim tick (same as GameState::tick)
        pierce_sim::construction::construction_system(&mut game.world);
        sim_tick(&mut game.world);
        crate::building::equip_factory_spawned_units(&mut game.world, &game.weapon_def_ids);
        crate::building::finalize_completed_buildings(&mut game.world);

        let checksum = world_checksum(&mut game.world);
        all_commands.push(frame_commands);
        all_checksums.push(checksum);
    }

    (all_commands, all_checksums)
}
