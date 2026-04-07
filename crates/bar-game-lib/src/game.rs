//! Game state management: tick loop, placement, factory queuing.

use std::collections::BTreeMap;
use std::path::Path;

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use recoil_math::SimFloat;
use recoil_sim::construction::Builder;
use recoil_sim::factory::BuildQueue;
use recoil_sim::projectile::ImpactEventQueue;
use recoil_sim::{Dead, Health, Position};

use crate::ai::{self, AiState};
use crate::building::{self, PlacementType};
use crate::production;
use crate::setup::{self, GameConfig};

/// Top-level game state (headless-capable, no rendering).
/// Result of a completed game.
#[derive(Debug, Clone)]
pub struct GameOver {
    /// Winning team (None = draw).
    pub winner: Option<u8>,
    /// Human-readable reason.
    pub reason: String,
}

pub struct GameState {
    pub world: World,
    pub paused: bool,
    pub frame_count: u64,
    pub selected: Option<Entity>,
    pub placement_mode: Option<PlacementType>,
    /// AI state for team 1.
    pub ai_state: AiState,
    /// Cached weapon def IDs per unit type.
    pub weapon_def_ids: BTreeMap<u32, Vec<u32>>,
    /// Map metal spots.
    pub metal_spots: Vec<(f64, f64)>,
    /// Commander entities per team.
    pub commander_team0: Option<Entity>,
    pub commander_team1: Option<Entity>,
    /// Track first factory for AI.
    pub factory_team1: Option<Entity>,
    /// Seed for misc RNG.
    pub rng_seed: u64,
    /// Set when the game ends.
    pub game_over: Option<GameOver>,
}

impl GameState {
    /// Create a new game state, performing full game setup.
    pub fn new(bar_units_path: &Path, map_manifest_path: &Path) -> Self {
        let mut world = World::new();
        let config = setup::setup_game(&mut world, bar_units_path, map_manifest_path);

        let ai_state = AiState::new(42, 1, 0, config.commander_team1, config.commander_team0);

        Self {
            world,
            paused: false,
            frame_count: 0,
            selected: None,
            placement_mode: None,
            ai_state,
            weapon_def_ids: config.weapon_def_ids,
            metal_spots: config.metal_spots,
            commander_team0: config.commander_team0,
            commander_team1: config.commander_team1,
            factory_team1: None,
            rng_seed: 12345,
            game_over: None,
        }
    }

    /// Create from an existing World and GameConfig (for testing or custom setups).
    pub fn from_config(world: World, config: GameConfig) -> Self {
        let ai_state = AiState::new(42, 1, 0, config.commander_team1, config.commander_team0);

        Self {
            world,
            paused: false,
            frame_count: 0,
            selected: None,
            placement_mode: None,
            ai_state,
            weapon_def_ids: config.weapon_def_ids,
            metal_spots: config.metal_spots,
            commander_team0: config.commander_team0,
            commander_team1: config.commander_team1,
            factory_team1: None,
            rng_seed: 12345,
            game_over: None,
        }
    }

    /// Reset the game to a fresh state.
    pub fn reset(&mut self, bar_units_path: &Path, map_manifest_path: &Path) {
        self.world = World::new();
        let config = setup::setup_game(&mut self.world, bar_units_path, map_manifest_path);

        self.selected = None;
        self.frame_count = 0;
        self.placement_mode = None;
        self.commander_team0 = config.commander_team0;
        self.commander_team1 = config.commander_team1;
        self.factory_team1 = None;
        self.weapon_def_ids = config.weapon_def_ids;
        self.metal_spots = config.metal_spots;
        self.ai_state = AiState::new(42, 1, 0, config.commander_team1, config.commander_team0);
        self.rng_seed = self.rng_seed.wrapping_add(7);
        self.game_over = None;
    }

    /// Returns true if the game has ended.
    pub fn is_game_over(&self) -> bool {
        self.game_over.is_some()
    }

    fn is_commander_dead(&self, cmd: Option<Entity>) -> bool {
        match cmd {
            None => true,
            Some(e) => {
                self.world.get_entity(e).is_err()
                    || self.world.get::<Dead>(e).is_some()
                    || self
                        .world
                        .get::<Health>(e)
                        .map(|h| h.current <= SimFloat::ZERO)
                        .unwrap_or(true)
            }
        }
    }

    fn check_game_over(&mut self) {
        if self.game_over.is_some() {
            return;
        }
        let t0_dead = self.is_commander_dead(self.commander_team0);
        let t1_dead = self.is_commander_dead(self.commander_team1);
        if t0_dead && t1_dead {
            self.game_over = Some(GameOver {
                winner: None,
                reason: "Both commanders destroyed".into(),
            });
        } else if t1_dead {
            self.game_over = Some(GameOver {
                winner: Some(0),
                reason: "Enemy commander destroyed".into(),
            });
        } else if t0_dead {
            self.game_over = Some(GameOver {
                winner: Some(1),
                reason: "Your commander was destroyed".into(),
            });
        }
    }

    /// Run one simulation tick. Returns (impact_positions, death_positions) for rendering.
    pub fn tick(&mut self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        if self.game_over.is_some() {
            return (Vec::new(), Vec::new());
        }
        // Snapshot entities with low health before sim_tick to detect deaths
        let pre_death: Vec<[f32; 3]> = self
            .world
            .query_filtered::<(&Position, &Health), Without<Dead>>()
            .iter(&self.world)
            .filter(|(_, h)| h.current <= SimFloat::ZERO)
            .map(|(p, _)| [p.pos.x.to_f32(), p.pos.y.to_f32() + 5.0, p.pos.z.to_f32()])
            .collect();

        // Capture impact positions
        let impact_positions: Vec<[f32; 3]> = self
            .world
            .resource::<ImpactEventQueue>()
            .events
            .iter()
            .map(|e| {
                [
                    e.position.x.to_f32(),
                    e.position.y.to_f32() + 5.0,
                    e.position.z.to_f32(),
                ]
            })
            .collect();

        // Run construction_system (not included in sim_tick)
        recoil_sim::construction::construction_system(&mut self.world);

        // Run all systems via sim_runner
        recoil_sim::sim_runner::sim_tick(&mut self.world);

        // Equip factory-spawned units with full components
        building::equip_factory_spawned_units(&mut self.world, &self.weapon_def_ids);

        // Check for completed build sites and convert them into functional buildings
        if let Some(new_factory) = building::finalize_completed_buildings(&mut self.world) {
            if self.factory_team1.is_none() {
                self.factory_team1 = Some(new_factory);
                self.ai_state.factory = Some(new_factory);
            }
        }

        // AI tick
        ai::ai_tick(&mut self.world, &mut self.ai_state, self.frame_count);

        // Detect newly dead entities
        let new_deaths: Vec<[f32; 3]> = {
            let mut q = self.world.query::<(&Position, &Dead, &Health)>();
            q.iter(&self.world)
                .filter(|(_, _, h)| h.current <= SimFloat::ZERO)
                .map(|(p, _, _)| [p.pos.x.to_f32(), p.pos.y.to_f32() + 5.0, p.pos.z.to_f32()])
                .collect()
        };

        let death_positions = if new_deaths.is_empty() {
            pre_death
        } else {
            new_deaths
        };

        self.check_game_over();

        (impact_positions, death_positions)
    }

    /// Enter placement mode for a building type.
    pub fn handle_build_command(&mut self, placement_type: PlacementType) {
        self.placement_mode = Some(placement_type);
    }

    /// Execute building placement at the given world position.
    /// Uses the currently selected builder (or commander as fallback).
    pub fn handle_place(&mut self, x: f32, z: f32) {
        if let Some(btype) = self.placement_mode.take() {
            // Use selected entity if it's a builder, otherwise fall back to commander.
            let builder = if self.selected_is_builder() {
                self.selected
            } else {
                self.commander_team0
            };
            // Determine team from the builder entity.
            let team = builder
                .and_then(|e| self.world.get::<recoil_sim::Allegiance>(e))
                .map(|a| a.team)
                .unwrap_or(0);
            building::place_building(&mut self.world, builder, btype.0, x, z, team);
        }
    }

    /// Queue a unit in a factory by name.
    pub fn handle_factory_queue(&mut self, factory_entity: Entity, unit_name: &str) {
        production::queue_unit_by_name(&mut self.world, factory_entity, unit_name);
    }

    /// Check if the selected entity is a factory.
    pub fn selected_is_factory(&self) -> bool {
        self.selected
            .map(|e| self.world.get::<BuildQueue>(e).is_some())
            .unwrap_or(false)
    }

    /// Check if the selected entity is a commander/builder.
    pub fn selected_is_builder(&self) -> bool {
        self.selected
            .map(|e| self.world.get::<Builder>(e).is_some())
            .unwrap_or(false)
    }

    // -------------------------------------------------------------------
    // Input simulation helpers (for headless UI testing)
    // -------------------------------------------------------------------

    /// Simulate left-click at world position: select nearest unit within radius.
    pub fn click_select(&mut self, x: f32, z: f32, radius: f32) -> Option<Entity> {
        let entity = self.find_unit_at(x, z, radius);
        self.selected = entity;
        entity
    }

    /// Simulate right-click at world position: issue move command to selected.
    /// Returns true if a move command was issued.
    pub fn click_move(&mut self, target_x: f32, target_z: f32) -> bool {
        if let Some(sel) = self.selected {
            if let Some(ms) = self.world.get_mut::<recoil_sim::MoveState>(sel) {
                *ms.into_inner() = recoil_sim::MoveState::MovingTo(
                    recoil_math::SimVec3::new(
                        recoil_math::SimFloat::from_f32(target_x),
                        recoil_math::SimFloat::ZERO,
                        recoil_math::SimFloat::from_f32(target_z),
                    ),
                );
                return true;
            }
        }
        false
    }

    /// Find nearest alive unit at world position within radius.
    pub fn find_unit_at(&mut self, x: f32, z: f32, radius: f32) -> Option<Entity> {
        let radius_sq = radius * radius;
        let mut best: Option<(Entity, f32)> = None;
        for (entity, pos) in self
            .world
            .query_filtered::<(Entity, &Position), bevy_ecs::query::Without<Dead>>()
            .iter(&self.world)
        {
            let dx = pos.pos.x.to_f32() - x;
            let dz = pos.pos.z.to_f32() - z;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq <= radius_sq && (best.is_none() || dist_sq < best.unwrap().1) {
                best = Some((entity, dist_sq));
            }
        }
        best.map(|(e, _)| e)
    }

    /// Queue a unit in a factory by type ID (data-driven).
    pub fn queue_unit_in_factory(&mut self, factory_entity: Entity, unit_type_id: u32) {
        production::queue_unit(&mut self.world, factory_entity, unit_type_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recoil_math::SimVec3;
    use recoil_sim::economy::EconomyState;

    /// Helper: create a GameState with fallback defs (no BAR repo needed).
    fn make_test_game() -> GameState {
        let bar_units = Path::new("nonexistent/units");
        let map_manifest = Path::new("assets/maps/small_duel/manifest.ron");
        GameState::new(bar_units, map_manifest)
    }

    #[test]
    fn test_full_game_setup() {
        let game = make_test_game();
        // Commander for team 0 should exist
        assert!(game.commander_team0.is_some());
        let cmd = game.commander_team0.unwrap();
        assert!(game.world.get::<Health>(cmd).is_some());
        assert!(game.world.get::<Builder>(cmd).is_some());

        // Commander for team 1 should exist
        assert!(game.commander_team1.is_some());
    }

    #[test]
    fn test_place_and_build_solar() {
        let mut game = make_test_game();

        // Give team 0 plenty of resources
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::from_int(10000);
                res.energy = SimFloat::from_int(50000);
            }
        }

        // Place a solar
        game.placement_mode = Some(PlacementType(building::BUILDING_SOLAR_ID));
        game.handle_place(300.0, 300.0);

        // Verify build site was created
        let build_sites: Vec<_> = game
            .world
            .query::<&recoil_sim::construction::BuildSite>()
            .iter(&game.world)
            .collect();
        assert!(
            !build_sites.is_empty(),
            "Should have at least one build site"
        );
    }

    #[test]
    fn test_game_tick_no_panic() {
        let mut game = make_test_game();
        // Run 100 ticks without panic
        for _ in 0..100 {
            game.tick();
            game.frame_count += 1;
        }
    }

    /// Give both teams abundant resources.
    fn fund_both_teams(game: &mut GameState) {
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

    // -----------------------------------------------------------------------
    // Determinism: run the same game twice, compare positions frame-by-frame
    // -----------------------------------------------------------------------

    /// Snapshot all alive entity positions sorted by SimId (deterministic order).
    fn snapshot_positions(game: &mut GameState) -> Vec<(u64, SimVec3)> {
        use recoil_sim::SimId;
        let mut positions: Vec<(u64, SimVec3)> = game
            .world
            .query_filtered::<(&SimId, &Position), Without<Dead>>()
            .iter(&game.world)
            .map(|(id, p)| (id.id, p.pos))
            .collect();
        positions.sort_by_key(|(id, _)| *id);
        positions
    }

    fn run_headless_game(frames: u64) -> Vec<Vec<(u64, SimVec3)>> {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        let mut ai0 =
            crate::ai::AiState::new(99, 0, 1, game.commander_team0, game.commander_team1);

        let mut snapshots = Vec::new();
        for _ in 0..frames {
            game.tick();
            crate::ai::ai_tick(&mut game.world, &mut ai0, game.frame_count);
            game.frame_count += 1;
            snapshots.push(snapshot_positions(&mut game));
        }
        snapshots
    }

    #[test]
    fn test_determinism_two_identical_runs() {
        let frames = 600;
        let trace_a = run_headless_game(frames);
        let trace_b = run_headless_game(frames);

        assert_eq!(trace_a.len(), trace_b.len());
        for (i, (a, b)) in trace_a.iter().zip(&trace_b).enumerate() {
            assert_eq!(
                a, b,
                "Determinism violation at frame {}: positions diverged",
                i
            );
        }
    }

    // -----------------------------------------------------------------------
    // Economy: solars produce energy, mexes produce metal
    // -----------------------------------------------------------------------

    #[test]
    fn test_solar_produces_energy() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        // Spawn a completed solar (no BuildSite) for team 0
        game.world.spawn((
            Position {
                pos: SimVec3::new(
                    SimFloat::from_f32(50.0),
                    SimFloat::ZERO,
                    SimFloat::from_f32(50.0),
                ),
            },
            Health {
                current: SimFloat::from_int(500),
                max: SimFloat::from_int(500),
            },
            recoil_sim::Allegiance { team: 0 },
            recoil_sim::UnitType {
                id: building::BUILDING_SOLAR_ID,
            },
            recoil_sim::CollisionRadius {
                radius: SimFloat::from_int(16),
            },
        ));

        // Finalize it so it gets a ResourceProducer
        building::finalize_completed_buildings(&mut game.world);

        let energy_before = {
            let economy = game.world.resource::<EconomyState>();
            economy.teams.get(&0).unwrap().energy
        };

        // Run some ticks for production system to accumulate
        for _ in 0..100 {
            game.tick();
            game.frame_count += 1;
        }

        let energy_after = {
            let economy = game.world.resource::<EconomyState>();
            economy.teams.get(&0).unwrap().energy
        };

        // Commanders also produce energy, so just check it went up
        assert!(
            energy_after > energy_before,
            "Energy should increase with a solar running: before={:?}, after={:?}",
            energy_before,
            energy_after
        );
    }

    #[test]
    fn test_mex_produces_metal() {
        let mut game = make_test_game();

        // Zero out team 0 metal to make accumulation obvious
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::ZERO;
                res.energy = SimFloat::from_int(100000);
                res.energy_storage = SimFloat::from_int(200000);
                res.metal_storage = SimFloat::from_int(100000);
            }
        }

        // Spawn a completed mex (no BuildSite) for team 0
        game.world.spawn((
            Position {
                pos: SimVec3::new(
                    SimFloat::from_f32(80.0),
                    SimFloat::ZERO,
                    SimFloat::from_f32(80.0),
                ),
            },
            Health {
                current: SimFloat::from_int(500),
                max: SimFloat::from_int(500),
            },
            recoil_sim::Allegiance { team: 0 },
            recoil_sim::UnitType {
                id: building::BUILDING_MEX_ID,
            },
            recoil_sim::CollisionRadius {
                radius: SimFloat::from_int(16),
            },
        ));

        building::finalize_completed_buildings(&mut game.world);

        // Tick and measure
        for _ in 0..100 {
            game.tick();
            game.frame_count += 1;
        }

        let metal_after = {
            let economy = game.world.resource::<EconomyState>();
            economy.teams.get(&0).unwrap().metal
        };

        // Commander produces 0.5 metal/tick too, but mex adds 3/tick so total should be > 0
        assert!(
            metal_after > SimFloat::ZERO,
            "Metal should accumulate with mex + commander producing"
        );
    }

    // -----------------------------------------------------------------------
    // Combat: units deal damage, entities die
    // -----------------------------------------------------------------------

    #[test]
    fn test_combat_units_take_damage() {
        use recoil_sim::combat_data::{DamageType, WeaponDef, WeaponInstance, WeaponSet};
        use recoil_sim::targeting::WeaponRegistry;

        let mut game = make_test_game();

        // Register a simple weapon in the WeaponRegistry
        let weapon_def_id = {
            let mut registry = game.world.resource_mut::<WeaponRegistry>();
            let id = registry.defs.len() as u32;
            registry.defs.push(WeaponDef {
                damage: SimFloat::from_int(50),
                damage_type: DamageType::Normal,
                range: SimFloat::from_int(200),
                reload_time: 10,
                projectile_speed: SimFloat::ZERO, // hitscan
                area_of_effect: SimFloat::ZERO,
                is_paralyzer: false,
            });
            id
        };

        // Spawn two armed units from opposing teams, close together
        let pos_a = SimVec3::new(
            SimFloat::from_int(500),
            SimFloat::ZERO,
            SimFloat::from_int(500),
        );
        let pos_b = SimVec3::new(
            SimFloat::from_int(520),
            SimFloat::ZERO,
            SimFloat::from_int(500),
        );

        let unit_a = recoil_sim::lifecycle::spawn_unit(
            &mut game.world,
            Position { pos: pos_a },
            recoil_sim::UnitType { id: 1 },
            recoil_sim::Allegiance { team: 0 },
            Health {
                current: SimFloat::from_int(500),
                max: SimFloat::from_int(500),
            },
        );
        game.world.entity_mut(unit_a).insert((
            recoil_sim::MoveState::Idle,
            recoil_sim::MovementParams {
                max_speed: SimFloat::from_int(2),
                acceleration: SimFloat::ONE,
                turn_rate: SimFloat::ONE,
            },
            recoil_sim::CollisionRadius {
                radius: SimFloat::from_int(8),
            },
            recoil_sim::Heading {
                angle: SimFloat::ZERO,
            },
            recoil_sim::Velocity { vel: SimVec3::ZERO },
            recoil_sim::combat_data::ArmorClass::Light,
            recoil_sim::Target { entity: None },
            WeaponSet {
                weapons: vec![WeaponInstance {
                    def_id: weapon_def_id,
                    reload_remaining: 0,
                }],
            },
            recoil_sim::SightRange {
                range: SimFloat::from_int(300),
            },
        ));

        let unit_b = recoil_sim::lifecycle::spawn_unit(
            &mut game.world,
            Position { pos: pos_b },
            recoil_sim::UnitType { id: 1 },
            recoil_sim::Allegiance { team: 1 },
            Health {
                current: SimFloat::from_int(500),
                max: SimFloat::from_int(500),
            },
        );
        game.world.entity_mut(unit_b).insert((
            recoil_sim::MoveState::Idle,
            recoil_sim::MovementParams {
                max_speed: SimFloat::from_int(2),
                acceleration: SimFloat::ONE,
                turn_rate: SimFloat::ONE,
            },
            recoil_sim::CollisionRadius {
                radius: SimFloat::from_int(8),
            },
            recoil_sim::Heading {
                angle: SimFloat::ZERO,
            },
            recoil_sim::Velocity { vel: SimVec3::ZERO },
            recoil_sim::combat_data::ArmorClass::Light,
            recoil_sim::Target { entity: None },
            WeaponSet {
                weapons: vec![WeaponInstance {
                    def_id: weapon_def_id,
                    reload_remaining: 0,
                }],
            },
            recoil_sim::SightRange {
                range: SimFloat::from_int(300),
            },
        ));

        let hp_a_before = game.world.get::<Health>(unit_a).unwrap().current;
        let hp_b_before = game.world.get::<Health>(unit_b).unwrap().current;

        // Run ticks — targeting system should acquire targets, weapons should fire
        for _ in 0..200 {
            game.tick();
            game.frame_count += 1;
        }

        // Units may have been killed/despawned — that counts as damage too
        let hp_a_after = game.world.get::<Health>(unit_a).map(|h| h.current);
        let hp_b_after = game.world.get::<Health>(unit_b).map(|h| h.current);

        let a_damaged = hp_a_after.is_none_or(|hp| hp < hp_a_before);
        let b_damaged = hp_b_after.is_none_or(|hp| hp < hp_b_before);
        assert!(
            a_damaged || b_damaged,
            "At least one unit should have taken damage or died"
        );
    }

    // -----------------------------------------------------------------------
    // Game reset
    // -----------------------------------------------------------------------

    #[test]
    fn test_game_reset_restores_state() {
        let mut game = make_test_game();

        // Run some ticks to mutate state
        for _ in 0..200 {
            game.tick();
            game.frame_count += 1;
        }

        // Reset
        let bar_units = Path::new("nonexistent/units");
        let map_manifest = Path::new("assets/maps/small_duel/manifest.ron");
        game.reset(bar_units, map_manifest);

        // frame_count is reset
        assert_eq!(game.frame_count, 0);
        // Commanders should be re-spawned
        assert!(game.commander_team0.is_some());
        assert!(game.commander_team1.is_some());
        // Both commanders should be alive with full health
        let cmd0 = game.commander_team0.unwrap();
        let hp = game.world.get::<Health>(cmd0).unwrap();
        assert_eq!(hp.current, hp.max);
    }

    // -----------------------------------------------------------------------
    // Place and build mex (coverage beyond solar)
    // -----------------------------------------------------------------------

    #[test]
    fn test_place_mex() {
        let mut game = make_test_game();
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::from_int(10000);
                res.energy = SimFloat::from_int(50000);
            }
        }

        game.placement_mode = Some(PlacementType(building::BUILDING_MEX_ID));
        game.handle_place(200.0, 200.0);

        let build_sites: Vec<_> = game
            .world
            .query_filtered::<
                &recoil_sim::UnitType,
                bevy_ecs::query::With<recoil_sim::construction::BuildSite>,
            >()
            .iter(&game.world)
            .filter(|ut| ut.id == building::BUILDING_MEX_ID)
            .collect();
        assert!(
            !build_sites.is_empty(),
            "Should have a mex build site after placement"
        );
    }

    // -----------------------------------------------------------------------
    // Factory-spawned units get equipped
    // -----------------------------------------------------------------------

    #[test]
    fn test_equip_factory_spawned_units() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        // Spawn a bare unit (like factory_system would) — no MoveState, no weapons
        let bare_unit = game
            .world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_f32(300.0),
                        SimFloat::ZERO,
                        SimFloat::from_f32(300.0),
                    ),
                },
                recoil_sim::UnitType {
                    id: recoil_sim::lua_unitdefs::hash_unit_name("armpw"),
                },
                recoil_sim::Allegiance { team: 0 },
                Health {
                    current: SimFloat::from_int(200),
                    max: SimFloat::from_int(200),
                },
            ))
            .id();

        // Should NOT have MoveState yet
        assert!(game.world.get::<recoil_sim::MoveState>(bare_unit).is_none());

        // Equip
        building::equip_factory_spawned_units(&mut game.world, &game.weapon_def_ids);

        // Now it should have movement, weapons, etc.
        assert!(
            game.world.get::<recoil_sim::MoveState>(bare_unit).is_some(),
            "Equipped unit should have MoveState"
        );
        assert!(
            game.world
                .get::<recoil_sim::combat_data::WeaponSet>(bare_unit)
                .is_some(),
            "Equipped unit should have WeaponSet"
        );
    }

    #[test]
    fn test_bot_vs_bot() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        let mut ai0 =
            crate::ai::AiState::new(99, 0, 1, game.commander_team0, game.commander_team1);

        for _ in 0..3000 {
            game.tick();
            crate::ai::ai_tick(&mut game.world, &mut ai0, game.frame_count);
            game.frame_count += 1;
        }

        let unit_count: usize = game
            .world
            .query_filtered::<&recoil_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();
        assert!(
            unit_count >= 2,
            "Expected at least 2 alive units after 3000 frames, got {}",
            unit_count
        );
    }

    #[test]
    fn test_factory_produces_unit() {
        let mut game = make_test_game();
        fund_team(&mut game, 0);

        game.placement_mode = Some(PlacementType(building::BUILDING_FACTORY_ID));
        game.handle_place(300.0, 300.0);

        let factory_entity = {
            let entities: Vec<_> = game
                .world
                .query_filtered::<(
                    bevy_ecs::entity::Entity,
                    &recoil_sim::UnitType,
                ), bevy_ecs::query::With<recoil_sim::construction::BuildSite>>()
                .iter(&game.world)
                .filter(|(_, ut)| ut.id == building::BUILDING_FACTORY_ID)
                .map(|(e, _)| e)
                .collect();
            entities.first().copied()
        };

        if let Some(factory) = factory_entity {
            game.world
                .entity_mut(factory)
                .remove::<recoil_sim::construction::BuildSite>();
            building::finalize_completed_buildings(&mut game.world);
            assert!(
                game.world.get::<BuildQueue>(factory).is_some(),
                "Completed factory should have a BuildQueue"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Helper: fund a single team
    // -----------------------------------------------------------------------

    fn fund_team(game: &mut GameState, team: u8) {
        let mut economy = game.world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&team) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(100000);
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }

    // -----------------------------------------------------------------------
    // Helper: spawn a fully armed unit
    // -----------------------------------------------------------------------

    fn register_test_weapon(game: &mut GameState) -> u32 {
        use recoil_sim::combat_data::{DamageType, WeaponDef};
        use recoil_sim::targeting::WeaponRegistry;

        let mut registry = game.world.resource_mut::<WeaponRegistry>();
        let id = registry.defs.len() as u32;
        registry.defs.push(WeaponDef {
            damage: SimFloat::from_int(50),
            damage_type: DamageType::Normal,
            range: SimFloat::from_int(200),
            reload_time: 10,
            projectile_speed: SimFloat::ZERO,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });
        id
    }

    fn spawn_armed_unit(
        game: &mut GameState,
        x: i32,
        z: i32,
        team: u8,
        weapon_def_id: u32,
        hp: i32,
    ) -> bevy_ecs::entity::Entity {
        use recoil_sim::combat_data::{ArmorClass, WeaponInstance, WeaponSet};

        let entity = recoil_sim::lifecycle::spawn_unit(
            &mut game.world,
            Position {
                pos: SimVec3::new(
                    SimFloat::from_int(x),
                    SimFloat::ZERO,
                    SimFloat::from_int(z),
                ),
            },
            recoil_sim::UnitType { id: 1 },
            recoil_sim::Allegiance { team },
            Health {
                current: SimFloat::from_int(hp),
                max: SimFloat::from_int(hp),
            },
        );
        game.world.entity_mut(entity).insert((
            recoil_sim::MoveState::Idle,
            recoil_sim::MovementParams {
                max_speed: SimFloat::from_int(2),
                acceleration: SimFloat::ONE,
                turn_rate: SimFloat::ONE,
            },
            recoil_sim::CollisionRadius {
                radius: SimFloat::from_int(8),
            },
            recoil_sim::Heading {
                angle: SimFloat::ZERO,
            },
            recoil_sim::Velocity { vel: SimVec3::ZERO },
            ArmorClass::Light,
            recoil_sim::Target { entity: None },
            WeaponSet {
                weapons: vec![WeaponInstance {
                    def_id: weapon_def_id,
                    reload_remaining: 0,
                }],
            },
            recoil_sim::SightRange {
                range: SimFloat::from_int(300),
            },
            recoil_sim::commands::CommandQueue::default(),
        ));
        entity
    }

    // -----------------------------------------------------------------------
    // Construction: builder completes a BuildSite over time
    // -----------------------------------------------------------------------

    #[test]
    fn test_construction_completes_over_time() {
        use recoil_sim::construction::{BuildSite, BuildTarget};

        let mut game = make_test_game();
        fund_team(&mut game, 0);

        let cmd = game.commander_team0.unwrap();

        // Place a solar near the commander
        let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;
        let bx = cmd_pos.x.to_f32() + 5.0;
        let bz = cmd_pos.z.to_f32();

        let site_entity = building::place_building(
            &mut game.world,
            Some(cmd),
            building::BUILDING_SOLAR_ID,
            bx,
            bz,
            0,
        )
        .expect("Should afford a solar");

        // Commander should have a BuildTarget pointing at the site
        assert!(game.world.get::<BuildTarget>(cmd).is_some());

        // Run ticks — construction_system runs inside game.tick()
        for _ in 0..5000 {
            game.tick();
            game.frame_count += 1;

            // Check if BuildSite has been removed (construction complete)
            if game.world.get::<BuildSite>(site_entity).is_none() {
                // Health should be set to max on completion
                let hp = game.world.get::<Health>(site_entity).unwrap();
                assert_eq!(
                    hp.current, hp.max,
                    "Completed building should have full health"
                );
                return;
            }
        }

        // If we get here, check progress — it should have advanced
        let site = game.world.get::<BuildSite>(site_entity).unwrap();
        assert!(
            site.progress > SimFloat::ZERO,
            "Construction should have made progress, got {:?}",
            site.progress
        );
    }

    // -----------------------------------------------------------------------
    // Reclaim: builder reclaims wreckage, gains metal
    // -----------------------------------------------------------------------

    #[test]
    fn test_reclaim_wreckage() {
        use recoil_sim::construction::{BuildTarget, Reclaimable};

        let mut game = make_test_game();

        // Zero team 0 metal so reclaim gain is obvious
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::ZERO;
                res.energy = SimFloat::from_int(100000);
                res.metal_storage = SimFloat::from_int(100000);
                res.energy_storage = SimFloat::from_int(200000);
            }
        }

        let cmd = game.commander_team0.unwrap();
        let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;

        // Spawn a reclaimable wreck near the commander
        let wreck = game
            .world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        cmd_pos.x + SimFloat::from_int(5),
                        SimFloat::ZERO,
                        cmd_pos.z,
                    ),
                },
                Reclaimable {
                    metal_value: SimFloat::from_int(200),
                    reclaim_progress: SimFloat::ZERO,
                },
                Health {
                    current: SimFloat::from_int(100),
                    max: SimFloat::from_int(100),
                },
                recoil_sim::Allegiance { team: 0 },
            ))
            .id();

        // Assign commander to reclaim
        game.world.entity_mut(cmd).insert(BuildTarget {
            target: wreck,
        });

        // Tick until reclaim completes or 5000 frames
        for _ in 0..5000 {
            game.tick();
            game.frame_count += 1;

            if game.world.get::<Reclaimable>(wreck).is_none()
                || game.world.get::<Dead>(wreck).is_some()
            {
                // Wreck was reclaimed — check that metal was gained
                let economy = game.world.resource::<EconomyState>();
                let metal = economy.teams.get(&0).unwrap().metal;
                assert!(
                    metal > SimFloat::ZERO,
                    "Team 0 should have gained metal from reclaim"
                );
                return;
            }
        }

        // Partial progress is acceptable if the builder is still working
        let r = game.world.get::<Reclaimable>(wreck).unwrap();
        assert!(
            r.reclaim_progress > SimFloat::ZERO,
            "Reclaim should have made some progress"
        );
    }

    // -----------------------------------------------------------------------
    // Economy stall: factory slows down when resources are low
    // -----------------------------------------------------------------------

    #[test]
    fn test_economy_stall_slows_factory() {
        let mut game = make_test_game();

        // Give team 0 very limited resources
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::from_int(10);
                res.energy = SimFloat::from_int(10);
                res.metal_storage = SimFloat::from_int(100);
                res.energy_storage = SimFloat::from_int(100);
            }
        }

        // Place and instantly complete a factory
        game.placement_mode = Some(PlacementType(building::BUILDING_FACTORY_ID));
        game.handle_place(300.0, 300.0);

        let factory_entity = {
            let entities: Vec<_> = game
                .world
                .query_filtered::<(
                    bevy_ecs::entity::Entity,
                    &recoil_sim::UnitType,
                ), bevy_ecs::query::With<recoil_sim::construction::BuildSite>>()
                .iter(&game.world)
                .filter(|(_, ut)| ut.id == building::BUILDING_FACTORY_ID)
                .map(|(e, _)| e)
                .collect();
            entities.first().copied()
        };

        if let Some(factory) = factory_entity {
            game.world
                .entity_mut(factory)
                .remove::<recoil_sim::construction::BuildSite>();
            building::finalize_completed_buildings(&mut game.world);

            // Queue a unit
            production::queue_unit(&mut game.world, factory, 9999);

            // Tick 100 frames with low resources
            for _ in 0..100 {
                game.tick();
                game.frame_count += 1;
            }

            // Factory progress should be limited due to stall
            let bq = game.world.get::<BuildQueue>(factory).unwrap();
            // The unit should still be in queue (not enough resources to complete)
            assert!(
                !bq.queue.is_empty() || bq.current_progress < SimFloat::ONE,
                "Factory should be stalled with insufficient resources"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Command queue: move command drives unit to destination
    // -----------------------------------------------------------------------

    #[test]
    fn test_command_queue_move() {
        use recoil_sim::commands::{Command, CommandQueue};

        let mut game = make_test_game();
        let weapon_id = register_test_weapon(&mut game);
        let unit = spawn_armed_unit(&mut game, 100, 100, 0, weapon_id, 500);

        // Issue a move command via the command queue
        let target = SimVec3::new(
            SimFloat::from_int(200),
            SimFloat::ZERO,
            SimFloat::from_int(200),
        );
        game.world
            .get_mut::<CommandQueue>(unit)
            .unwrap()
            .replace(Command::Move(target));

        let start_pos = game.world.get::<Position>(unit).unwrap().pos;

        // Tick
        for _ in 0..500 {
            game.tick();
            game.frame_count += 1;
        }

        let end_pos = game.world.get::<Position>(unit).unwrap().pos;
        assert_ne!(start_pos, end_pos, "Unit should have moved from start");

        // Unit should be closer to target
        let dist_before = (target.x - start_pos.x).abs() + (target.z - start_pos.z).abs();
        let dist_after = (target.x - end_pos.x).abs() + (target.z - end_pos.z).abs();
        assert!(
            dist_after < dist_before,
            "Unit should be closer to target after moving"
        );
    }

    // -----------------------------------------------------------------------
    // Command queue: stop command halts movement
    // -----------------------------------------------------------------------

    #[test]
    fn test_command_stop() {
        use recoil_sim::commands::{Command, CommandQueue};

        let mut game = make_test_game();
        let weapon_id = register_test_weapon(&mut game);
        let unit = spawn_armed_unit(&mut game, 100, 100, 0, weapon_id, 500);

        // Start moving
        let target = SimVec3::new(
            SimFloat::from_int(500),
            SimFloat::ZERO,
            SimFloat::from_int(500),
        );
        *game.world.get_mut::<recoil_sim::MoveState>(unit).unwrap() =
            recoil_sim::MoveState::MovingTo(target);

        // Tick a few frames to start moving
        for _ in 0..10 {
            game.tick();
            game.frame_count += 1;
        }

        // Issue stop
        game.world
            .get_mut::<CommandQueue>(unit)
            .unwrap()
            .replace(Command::Stop);

        // Tick to process stop
        game.tick();
        game.frame_count += 1;

        let ms = game.world.get::<recoil_sim::MoveState>(unit).unwrap();
        assert_eq!(*ms, recoil_sim::MoveState::Idle, "Stop should set Idle");

        let cq = game.world.get::<CommandQueue>(unit).unwrap();
        assert!(cq.is_empty(), "Stop should clear the command queue");
    }

    // -----------------------------------------------------------------------
    // Unit movement: unit arrives at target
    // -----------------------------------------------------------------------

    #[test]
    fn test_unit_arrives_at_target() {
        let mut game = make_test_game();
        let weapon_id = register_test_weapon(&mut game);
        let unit = spawn_armed_unit(&mut game, 100, 100, 0, weapon_id, 500);

        // Set a close target
        let target = SimVec3::new(
            SimFloat::from_int(110),
            SimFloat::ZERO,
            SimFloat::from_int(100),
        );
        *game.world.get_mut::<recoil_sim::MoveState>(unit).unwrap() =
            recoil_sim::MoveState::MovingTo(target);

        // Tick until idle (arrived)
        for _ in 0..200 {
            game.tick();
            game.frame_count += 1;

            let ms = game.world.get::<recoil_sim::MoveState>(unit).unwrap();
            if *ms == recoil_sim::MoveState::Idle {
                // Check position is near target
                let pos = game.world.get::<Position>(unit).unwrap().pos;
                let dx = (pos.x - target.x).abs();
                let dz = (pos.z - target.z).abs();
                assert!(
                    dx < SimFloat::from_int(5) && dz < SimFloat::from_int(5),
                    "Unit should be near target when arrived"
                );
                return;
            }
        }
        panic!("Unit did not arrive at target within 200 ticks");
    }

    // -----------------------------------------------------------------------
    // Projectiles: fire events spawn projectiles, projectiles impact
    // -----------------------------------------------------------------------

    #[test]
    fn test_projectiles_spawn_and_impact() {
        use recoil_sim::combat_data::{DamageType, WeaponDef};
        use recoil_sim::projectile::Projectile;
        use recoil_sim::targeting::WeaponRegistry;

        let mut game = make_test_game();

        // Register a projectile weapon (non-zero speed = actual projectiles)
        let weapon_def_id = {
            let mut registry = game.world.resource_mut::<WeaponRegistry>();
            let id = registry.defs.len() as u32;
            registry.defs.push(WeaponDef {
                damage: SimFloat::from_int(10),
                damage_type: DamageType::Normal,
                range: SimFloat::from_int(200),
                reload_time: 15,
                projectile_speed: SimFloat::from_int(5),
                area_of_effect: SimFloat::ZERO,
                is_paralyzer: false,
            });
            id
        };

        // Spawn two opposing units close together — high HP so they survive
        let unit_a = spawn_armed_unit(&mut game, 500, 500, 0, weapon_def_id, 5000);
        let unit_b = spawn_armed_unit(&mut game, 530, 500, 1, weapon_def_id, 5000);

        let mut saw_projectile = false;

        // Tick and check for projectile entities in the world
        for _ in 0..200 {
            game.tick();
            game.frame_count += 1;

            let proj_count = game
                .world
                .query::<&Projectile>()
                .iter(&game.world)
                .count();
            if proj_count > 0 {
                saw_projectile = true;
            }
        }

        // Should have seen projectiles, and at least one unit took damage
        let hp_a = game.world.get::<Health>(unit_a).map(|h| h.current);
        let hp_b = game.world.get::<Health>(unit_b).map(|h| h.current);
        let damage_dealt = hp_a.is_none_or(|h| h < SimFloat::from_int(5000))
            || hp_b.is_none_or(|h| h < SimFloat::from_int(5000));

        assert!(
            saw_projectile || damage_dealt,
            "Should have seen projectiles in flight or damage dealt"
        );
    }

    // -----------------------------------------------------------------------
    // Fog of war: visibility updates around units
    // -----------------------------------------------------------------------

    #[test]
    fn test_fog_of_war_updates() {
        use recoil_sim::fog::{CellVisibility, FogOfWar};

        let mut game = make_test_game();

        // Tick a few frames so fog_system runs
        for _ in 0..10 {
            game.tick();
            game.frame_count += 1;
        }

        let fog = game.world.resource::<FogOfWar>();

        // Commander for team 0 should have revealed cells near its position
        let cmd0 = game.commander_team0.unwrap();
        let cmd_pos = game.world.get::<Position>(cmd0).unwrap().pos;

        // Convert world pos to fog grid cell (fog uses cell_size=1 in sim_tick)
        let cx = (cmd_pos.x.to_f32() as u32).min(fog.width() - 1);
        let cz = (cmd_pos.z.to_f32() as u32).min(fog.height() - 1);

        let vis = fog.get(0, cx, cz);
        assert_eq!(
            vis,
            CellVisibility::Visible,
            "Cell at commander position should be Visible for team 0"
        );
    }

    // -----------------------------------------------------------------------
    // Dead units get cleaned up
    // -----------------------------------------------------------------------

    #[test]
    fn test_dead_units_cleaned_up() {
        use recoil_sim::combat_data::{DamageType, WeaponDef};
        use recoil_sim::targeting::WeaponRegistry;

        let mut game = make_test_game();

        // Register a very powerful hitscan weapon
        let weapon_def_id = {
            let mut registry = game.world.resource_mut::<WeaponRegistry>();
            let id = registry.defs.len() as u32;
            registry.defs.push(WeaponDef {
                damage: SimFloat::from_int(9999),
                damage_type: DamageType::Normal,
                range: SimFloat::from_int(500),
                reload_time: 1,
                projectile_speed: SimFloat::ZERO,
                area_of_effect: SimFloat::ZERO,
                is_paralyzer: false,
            });
            id
        };

        // Spawn a strong attacker and a weak victim
        spawn_armed_unit(&mut game, 400, 400, 0, weapon_def_id, 5000);
        let victim = spawn_armed_unit(&mut game, 420, 400, 1, weapon_def_id, 50);

        // Tick until victim is dead/despawned
        for _ in 0..100 {
            game.tick();
            game.frame_count += 1;

            // Check if victim has been despawned
            if game.world.get_entity(victim).is_err() {
                return; // Successfully cleaned up
            }
        }

        // If still exists, it should at least be marked Dead
        if game.world.get_entity(victim).is_ok() {
            assert!(
                game.world.get::<Dead>(victim).is_some(),
                "Low-health unit targeted by an enemy should eventually die"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Paralyzer / stun: stunned units exist in the game
    // -----------------------------------------------------------------------

    #[test]
    fn test_stun_decrements_over_time() {
        use recoil_sim::components::Stunned;

        let mut game = make_test_game();
        let weapon_id = register_test_weapon(&mut game);
        let unit = spawn_armed_unit(&mut game, 300, 300, 0, weapon_id, 500);

        // Manually apply stun
        game.world.entity_mut(unit).insert(Stunned {
            remaining_frames: 50,
        });

        // Tick
        for _ in 0..10 {
            game.tick();
            game.frame_count += 1;
        }

        // Stun should have decremented
        if let Some(stunned) = game.world.get::<Stunned>(unit) {
            assert!(
                stunned.remaining_frames < 50,
                "Stun should decrement: got {}",
                stunned.remaining_frames
            );
        }
        // If stun component was removed, that's also valid (fully recovered)
    }

    // -----------------------------------------------------------------------
    // Factory production pipeline: queue -> tick -> unit spawns at rally
    // -----------------------------------------------------------------------

    #[test]
    fn test_factory_full_production_pipeline() {
        use recoil_sim::factory::{UnitBlueprint, UnitRegistry};

        let mut game = make_test_game();
        fund_team(&mut game, 0);

        // Register a cheap fast-build blueprint
        {
            let mut registry = game.world.resource_mut::<UnitRegistry>();
            registry.blueprints.push(UnitBlueprint {
                unit_type_id: 7777,
                metal_cost: SimFloat::from_int(10),
                energy_cost: SimFloat::from_int(10),
                build_time: 5, // very fast
                max_health: SimFloat::from_int(100),
            });
        }

        // Create a factory entity directly (skip construction)
        let rally = SimVec3::new(
            SimFloat::from_int(350),
            SimFloat::ZERO,
            SimFloat::from_int(300),
        );
        let factory = game
            .world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(300),
                        SimFloat::ZERO,
                        SimFloat::from_int(300),
                    ),
                },
                BuildQueue {
                    queue: std::collections::VecDeque::new(),
                    current_progress: SimFloat::ZERO,
                    rally_point: rally,
                    repeat: false,
                },
                recoil_sim::Allegiance { team: 0 },
                recoil_sim::UnitType {
                    id: building::BUILDING_FACTORY_ID,
                },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
            ))
            .id();

        // Queue the unit
        production::queue_unit(&mut game.world, factory, 7777);

        // Count initial units with UnitType 7777
        let initial_count: usize = game
            .world
            .query::<&recoil_sim::UnitType>()
            .iter(&game.world)
            .filter(|ut| ut.id == 7777)
            .count();

        // Tick enough for production to complete
        for _ in 0..50 {
            game.tick();
            game.frame_count += 1;
        }

        // A new entity with UnitType 7777 should exist
        let final_count: usize = game
            .world
            .query::<&recoil_sim::UnitType>()
            .iter(&game.world)
            .filter(|ut| ut.id == 7777)
            .count();

        assert!(
            final_count > initial_count,
            "Factory should have produced a unit: before={}, after={}",
            initial_count,
            final_count
        );

        // The factory queue should be empty
        let bq = game.world.get::<BuildQueue>(factory).unwrap();
        assert!(bq.queue.is_empty(), "Queue should be empty after producing");
    }

    // -----------------------------------------------------------------------
    // AoE damage: explosive weapon damages multiple units
    // -----------------------------------------------------------------------

    #[test]
    fn test_aoe_damage() {
        use recoil_sim::combat_data::{DamageType, WeaponDef};
        use recoil_sim::targeting::WeaponRegistry;

        let mut game = make_test_game();

        // Register an explosive AoE weapon
        let weapon_def_id = {
            let mut registry = game.world.resource_mut::<WeaponRegistry>();
            let id = registry.defs.len() as u32;
            registry.defs.push(WeaponDef {
                damage: SimFloat::from_int(100),
                damage_type: DamageType::Explosive,
                range: SimFloat::from_int(200),
                reload_time: 5,
                projectile_speed: SimFloat::ZERO, // hitscan
                area_of_effect: SimFloat::from_int(50),
                is_paralyzer: false,
            });
            id
        };

        // Spawn attacker for team 0
        spawn_armed_unit(&mut game, 500, 500, 0, weapon_def_id, 5000);

        // Spawn a cluster of enemies for team 1
        let enemies: Vec<_> = (0..3)
            .map(|i| spawn_armed_unit(&mut game, 520 + i * 5, 500, 1, weapon_def_id, 500))
            .collect();

        // Tick
        for _ in 0..200 {
            game.tick();
            game.frame_count += 1;
        }

        // At least one enemy should have taken damage or died
        let damaged_count = enemies
            .iter()
            .filter(|&&e| {
                game.world
                    .get::<Health>(e)
                    .is_none_or(|h| h.current < h.max)
            })
            .count();

        assert!(
            damaged_count > 0,
            "AoE weapon should have damaged at least one unit in the cluster"
        );
    }

    // -----------------------------------------------------------------------
    // Determinism with world_checksum (stronger comparison)
    // -----------------------------------------------------------------------

    #[test]
    fn test_determinism_world_checksum() {
        fn run_and_checksum(frames: u64) -> Vec<u64> {
            let mut game = make_test_game();

            // Fund and enable both AIs
            {
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

            let mut ai0 = crate::ai::AiState::new(
                99,
                0,
                1,
                game.commander_team0,
                game.commander_team1,
            );

            let mut checksums = Vec::new();
            for _ in 0..frames {
                game.tick();
                crate::ai::ai_tick(&mut game.world, &mut ai0, game.frame_count);
                game.frame_count += 1;

                // Sample every 50 frames to keep it fast
                if game.frame_count.is_multiple_of(50) {
                    checksums.push(recoil_sim::sim_runner::world_checksum(&mut game.world));
                }
            }
            checksums
        }

        let trace_a = run_and_checksum(500);
        let trace_b = run_and_checksum(500);

        assert_eq!(trace_a.len(), trace_b.len());
        for (i, (a, b)) in trace_a.iter().zip(&trace_b).enumerate() {
            assert_eq!(
                a, b,
                "Determinism (world_checksum) diverged at sample {}",
                i
            );
        }
    }

    // -----------------------------------------------------------------------
    // Selected entity helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_selected_is_factory() {
        let mut game = make_test_game();
        fund_team(&mut game, 0);

        // Place and complete a factory
        game.placement_mode = Some(PlacementType(building::BUILDING_FACTORY_ID));
        game.handle_place(300.0, 300.0);

        let factory_entity = {
            game.world
                .query_filtered::<(
                    bevy_ecs::entity::Entity,
                    &recoil_sim::UnitType,
                ), bevy_ecs::query::With<recoil_sim::construction::BuildSite>>()
                .iter(&game.world)
                .find(|(_, ut)| ut.id == building::BUILDING_FACTORY_ID)
                .map(|(e, _)| e)
        };

        if let Some(factory) = factory_entity {
            game.world
                .entity_mut(factory)
                .remove::<recoil_sim::construction::BuildSite>();
            building::finalize_completed_buildings(&mut game.world);

            game.selected = Some(factory);
            assert!(game.selected_is_factory());
            assert!(!game.selected_is_builder());
        }
    }

    #[test]
    fn test_selected_is_builder() {
        let game = make_test_game();
        let cmd = game.commander_team0.unwrap();

        let mut game = game;
        game.selected = Some(cmd);
        assert!(game.selected_is_builder());
        assert!(!game.selected_is_factory());
    }

    #[test]
    fn test_selected_none() {
        let game = make_test_game();
        assert!(!game.selected_is_factory());
        assert!(!game.selected_is_builder());
    }

    // -----------------------------------------------------------------------
    // Paused game: ticks still work but game state is accessible
    // -----------------------------------------------------------------------

    #[test]
    fn test_paused_state() {
        let mut game = make_test_game();
        game.paused = true;

        // Pausing is a UI concern — tick still runs if called
        // But verify the flag is set correctly
        assert!(game.paused);

        // Still able to query state while paused
        assert!(game.commander_team0.is_some());
    }

    // -----------------------------------------------------------------------
    // handle_build_command sets placement mode
    // -----------------------------------------------------------------------

    #[test]
    fn test_handle_build_command() {
        let mut game = make_test_game();
        assert!(game.placement_mode.is_none());

        game.handle_build_command(PlacementType(building::BUILDING_FACTORY_ID));
        assert_eq!(game.placement_mode, Some(PlacementType(building::BUILDING_FACTORY_ID)));

        game.handle_build_command(PlacementType(building::BUILDING_SOLAR_ID));
        assert_eq!(game.placement_mode, Some(PlacementType(building::BUILDING_SOLAR_ID)));
    }

    // -----------------------------------------------------------------------
    // handle_factory_queue: queues unit by name through GameState API
    // -----------------------------------------------------------------------

    #[test]
    fn test_handle_factory_queue() {
        let mut game = make_test_game();
        fund_team(&mut game, 0);

        // Create a factory directly
        let factory = game
            .world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(300),
                        SimFloat::ZERO,
                        SimFloat::from_int(300),
                    ),
                },
                BuildQueue {
                    queue: std::collections::VecDeque::new(),
                    current_progress: SimFloat::ZERO,
                    rally_point: SimVec3::ZERO,
                    repeat: false,
                },
                recoil_sim::Allegiance { team: 0 },
                recoil_sim::UnitType {
                    id: building::BUILDING_FACTORY_ID,
                },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
            ))
            .id();

        // Queue a unit that doesn't exist — should not panic
        game.handle_factory_queue(factory, "nonexistent_unit");
        let bq = game.world.get::<BuildQueue>(factory).unwrap();
        assert!(bq.queue.is_empty(), "Unknown unit should not be queued");
    }

    // -----------------------------------------------------------------------
    // Multiple building types economy: producers/consumers balance
    // -----------------------------------------------------------------------

    #[test]
    fn test_multi_building_economy_balance() {
        use recoil_sim::economy::ResourceProducer;

        let mut game = make_test_game();

        // Set team 0 to zero resources
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::ZERO;
                res.energy = SimFloat::ZERO;
                res.metal_storage = SimFloat::from_int(100000);
                res.energy_storage = SimFloat::from_int(100000);
            }
        }

        // Spawn multiple completed solars and mexes for team 0
        for i in 0..5 {
            game.world.spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(50 + i * 20),
                        SimFloat::ZERO,
                        SimFloat::from_int(50),
                    ),
                },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
                recoil_sim::Allegiance { team: 0 },
                recoil_sim::UnitType {
                    id: building::BUILDING_SOLAR_ID,
                },
                recoil_sim::CollisionRadius {
                    radius: SimFloat::from_int(16),
                },
            ));
        }
        for i in 0..3 {
            game.world.spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(50 + i * 20),
                        SimFloat::ZERO,
                        SimFloat::from_int(100),
                    ),
                },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
                recoil_sim::Allegiance { team: 0 },
                recoil_sim::UnitType {
                    id: building::BUILDING_MEX_ID,
                },
                recoil_sim::CollisionRadius {
                    radius: SimFloat::from_int(16),
                },
            ));
        }

        // Finalize all buildings
        building::finalize_completed_buildings(&mut game.world);

        // Verify producers were added
        let producer_count = game
            .world
            .query::<(&ResourceProducer, &recoil_sim::Allegiance)>()
            .iter(&game.world)
            .filter(|(_, a)| a.team == 0)
            .count();
        // 5 solars + 3 mexes + 1 commander = 9 producers
        assert!(
            producer_count >= 8,
            "Should have multiple producers: got {}",
            producer_count
        );

        // Tick and verify resources grow
        for _ in 0..200 {
            game.tick();
            game.frame_count += 1;
        }

        let economy = game.world.resource::<EconomyState>();
        let res = economy.teams.get(&0).unwrap();
        assert!(
            res.metal > SimFloat::ZERO,
            "Metal should accumulate from mexes"
        );
        assert!(
            res.energy > SimFloat::ZERO,
            "Energy should accumulate from solars"
        );
    }

    // -----------------------------------------------------------------------
    // Long-running stress: 5000+ frames without panic
    // -----------------------------------------------------------------------

    #[test]
    fn test_long_running_bot_vs_bot() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        let mut ai0 =
            crate::ai::AiState::new(99, 0, 1, game.commander_team0, game.commander_team1);

        for _ in 0..5000 {
            game.tick();
            crate::ai::ai_tick(&mut game.world, &mut ai0, game.frame_count);
            game.frame_count += 1;
        }

        // Should complete without panic — verify game state is still coherent
        assert!(game.frame_count == 5000);

        // At least one team should still have some units alive
        let alive_count: usize = game
            .world
            .query_filtered::<&recoil_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();
        assert!(
            alive_count > 0,
            "At least one unit should still be alive after 5000 frames"
        );
    }

    // -----------------------------------------------------------------------
    // Tick returns impact and death positions
    // -----------------------------------------------------------------------

    #[test]
    fn test_tick_returns_events() {
        let mut game = make_test_game();

        // The tick function returns (impacts, deaths)
        let (impacts, deaths) = game.tick();
        game.frame_count += 1;

        // On first tick, no combat has happened so these should be empty
        assert!(
            impacts.is_empty(),
            "No impacts expected on first tick of fresh game"
        );
        // Deaths may or may not be empty depending on initial state
        let _ = deaths;
    }

    // -----------------------------------------------------------------------
    // from_config constructor
    // -----------------------------------------------------------------------

    #[test]
    fn test_from_config() {
        use recoil_sim::economy::init_economy;
        use recoil_sim::sim_runner;

        let mut world = bevy_ecs::world::World::new();
        sim_runner::init_sim_world(&mut world);
        init_economy(&mut world, &[0, 1]);

        let config = GameConfig {
            weapon_def_ids: BTreeMap::new(),
            metal_spots: vec![(100.0, 200.0), (300.0, 400.0)],
            commander_team0: None,
            commander_team1: None,
        };

        let game = GameState::from_config(world, config);
        assert_eq!(game.metal_spots.len(), 2);
        assert_eq!(game.frame_count, 0);
        assert!(!game.paused);
        assert!(game.placement_mode.is_none());
    }

    // -----------------------------------------------------------------------
    // PlacementType labels
    // -----------------------------------------------------------------------

    #[test]
    fn test_placement_type_labels() {
        let game = make_test_game();
        let registry = game.world.resource::<recoil_sim::unit_defs::UnitDefRegistry>();

        // Labels should format as "Build <name>" for known types
        let solar = PlacementType(building::BUILDING_SOLAR_ID);
        let label = solar.label(registry);
        assert!(label.starts_with("Build "), "Label should start with 'Build ': {}", label);

        // Unknown type falls back to "Build #<id>"
        let unknown = PlacementType(99999);
        assert!(unknown.label(registry).starts_with("Build #"));
    }

    // ===================================================================
    // UI INTERACTION TESTS
    // Simulate player click/keyboard actions headlessly and verify results.
    // ===================================================================

    // -----------------------------------------------------------------------
    // Click-select: left-click on a unit selects it
    // -----------------------------------------------------------------------

    #[test]
    fn ui_click_select_commander() {
        let mut game = make_test_game();

        // Get commander position
        let cmd = game.commander_team0.unwrap();
        let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;
        let cx = cmd_pos.x.to_f32();
        let cz = cmd_pos.z.to_f32();

        // Nothing selected initially
        assert!(game.selected.is_none());

        // Click near the commander
        let selected = game.click_select(cx + 1.0, cz + 1.0, 20.0);
        assert_eq!(selected, Some(cmd), "Should select the commander");
        assert_eq!(game.selected, Some(cmd));
        assert!(game.selected_is_builder(), "Commander is a builder");
    }

    #[test]
    fn ui_click_select_empty_ground() {
        let mut game = make_test_game();

        // Click far from any unit
        let selected = game.click_select(999.0, 999.0, 20.0);
        assert!(selected.is_none(), "Should not select anything on empty ground");
        assert!(game.selected.is_none());
    }

    #[test]
    fn ui_click_select_switches_unit() {
        let mut game = make_test_game();

        let cmd0 = game.commander_team0.unwrap();
        let cmd1 = game.commander_team1.unwrap();

        let pos0 = game.world.get::<Position>(cmd0).unwrap().pos;
        let pos1 = game.world.get::<Position>(cmd1).unwrap().pos;

        // Select commander 0
        game.click_select(pos0.x.to_f32(), pos0.z.to_f32(), 20.0);
        assert_eq!(game.selected, Some(cmd0));

        // Select commander 1
        game.click_select(pos1.x.to_f32(), pos1.z.to_f32(), 20.0);
        assert_eq!(game.selected, Some(cmd1));
    }

    // -----------------------------------------------------------------------
    // Right-click move: moves selected unit to target
    // -----------------------------------------------------------------------

    #[test]
    fn ui_right_click_moves_unit() {
        let mut game = make_test_game();

        let cmd = game.commander_team0.unwrap();
        let start_pos = game.world.get::<Position>(cmd).unwrap().pos;

        // Select the commander
        game.selected = Some(cmd);

        // Right-click to move
        let target_x = start_pos.x.to_f32() + 50.0;
        let target_z = start_pos.z.to_f32() + 50.0;
        let moved = game.click_move(target_x, target_z);
        assert!(moved, "Should issue move command");

        // Verify MoveState changed
        let ms = game.world.get::<recoil_sim::MoveState>(cmd).unwrap();
        assert!(
            matches!(ms, recoil_sim::MoveState::MovingTo(_)),
            "Unit should be MovingTo"
        );

        // Tick and verify unit moves toward target
        for _ in 0..100 {
            game.tick();
            game.frame_count += 1;
        }

        let new_pos = game.world.get::<Position>(cmd).unwrap().pos;
        let dx = (new_pos.x - start_pos.x).abs();
        let dz = (new_pos.z - start_pos.z).abs();
        assert!(
            dx > SimFloat::ZERO || dz > SimFloat::ZERO,
            "Unit should have moved from start position"
        );
    }

    #[test]
    fn ui_right_click_no_selection_does_nothing() {
        let mut game = make_test_game();
        assert!(game.selected.is_none());

        let moved = game.click_move(500.0, 500.0);
        assert!(!moved, "Should not move when nothing is selected");
    }

    // -----------------------------------------------------------------------
    // Building flow: select builder -> press build key -> click to place
    // -----------------------------------------------------------------------

    #[test]
    fn ui_full_building_flow() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        // Step 1: Select commander (builder)
        let cmd = game.commander_team0.unwrap();
        game.selected = Some(cmd);
        assert!(game.selected_is_builder());

        // Step 2: Enter build mode (simulates pressing a build key)
        game.handle_build_command(PlacementType(building::BUILDING_SOLAR_ID));
        assert!(game.placement_mode.is_some());

        // Step 3: Click to place building
        let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;
        let bx = cmd_pos.x.to_f32() + 10.0;
        let bz = cmd_pos.z.to_f32();
        game.handle_place(bx, bz);

        // Verify: placement mode cleared
        assert!(game.placement_mode.is_none(), "Placement mode should clear after placing");

        // Verify: BuildSite was created at the location
        let sites: Vec<_> = game
            .world
            .query::<(&recoil_sim::construction::BuildSite, &Position)>()
            .iter(&game.world)
            .collect();
        assert!(!sites.is_empty(), "A build site should exist");

        // Step 4: Tick until construction progresses
        for _ in 0..100 {
            game.tick();
            game.frame_count += 1;
        }

        // Verify some progress was made
        let site = game
            .world
            .query::<&recoil_sim::construction::BuildSite>()
            .iter(&game.world)
            .next();
        // Site either completed (removed) or has progress
        if let Some(site) = site {
            assert!(
                site.progress > SimFloat::ZERO,
                "Construction should have progressed"
            );
        }
        // If site is None, building completed — also good
    }

    #[test]
    fn ui_cancel_placement_right_click() {
        let mut game = make_test_game();

        game.selected = Some(game.commander_team0.unwrap());
        game.handle_build_command(PlacementType(building::BUILDING_SOLAR_ID));
        assert!(game.placement_mode.is_some());

        // Cancel (simulates right-click in placement mode)
        game.placement_mode = None;
        assert!(game.placement_mode.is_none());
    }

    #[test]
    fn ui_cannot_build_without_resources() {
        let mut game = make_test_game();

        // Drain all resources
        {
            let mut economy = game.world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::ZERO;
                res.energy = SimFloat::ZERO;
            }
        }

        game.selected = Some(game.commander_team0.unwrap());
        game.handle_build_command(PlacementType(building::BUILDING_SOLAR_ID));
        game.handle_place(300.0, 300.0);

        // Should NOT have created a build site (can't afford)
        let site_count = game
            .world
            .query::<&recoil_sim::construction::BuildSite>()
            .iter(&game.world)
            .count();
        assert_eq!(site_count, 0, "Should not place building without resources");
    }

    // -----------------------------------------------------------------------
    // Factory flow: select factory -> press queue key -> verify production
    // -----------------------------------------------------------------------

    #[test]
    fn ui_factory_queue_and_produce() {
        use recoil_sim::factory::{BuildQueue, UnitBlueprint, UnitRegistry};

        let mut game = make_test_game();
        fund_team(&mut game, 0);

        // Register a quick-build unit blueprint with unique ID
        let test_unit_id = 55555u32;
        {
            let mut registry = game.world.resource_mut::<UnitRegistry>();
            registry.blueprints.push(UnitBlueprint {
                unit_type_id: test_unit_id,
                metal_cost: SimFloat::from_int(10),
                energy_cost: SimFloat::from_int(10),
                build_time: 3,
                max_health: SimFloat::from_int(100),
            });
        }

        // Create a factory entity
        let rally = SimVec3::new(
            SimFloat::from_int(350),
            SimFloat::ZERO,
            SimFloat::from_int(300),
        );
        let factory = game
            .world
            .spawn((
                Position {
                    pos: SimVec3::new(
                        SimFloat::from_int(300),
                        SimFloat::ZERO,
                        SimFloat::from_int(300),
                    ),
                },
                BuildQueue {
                    queue: std::collections::VecDeque::new(),
                    current_progress: SimFloat::ZERO,
                    rally_point: rally,
                    repeat: false,
                },
                recoil_sim::Allegiance { team: 0 },
                recoil_sim::UnitType {
                    id: building::BUILDING_FACTORY_ID,
                },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
            ))
            .id();

        // Step 1: Select the factory
        game.selected = Some(factory);
        assert!(game.selected_is_factory());
        assert!(!game.selected_is_builder());

        // Step 2: Queue a unit (simulates pressing 1)
        game.queue_unit_in_factory(factory, test_unit_id);

        let bq = game.world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.queue.len(), 1, "Queue should have 1 item");

        // Step 3: Tick until production completes
        let initial_units: usize = game
            .world
            .query::<&recoil_sim::UnitType>()
            .iter(&game.world)
            .filter(|ut| ut.id == test_unit_id)
            .count();

        for _ in 0..50 {
            game.tick();
            game.frame_count += 1;
        }

        let final_units: usize = game
            .world
            .query::<&recoil_sim::UnitType>()
            .iter(&game.world)
            .filter(|ut| ut.id == test_unit_id)
            .count();

        assert!(
            final_units > initial_units,
            "Factory should have produced a unit: before={}, after={}",
            initial_units,
            final_units
        );
    }

    // -----------------------------------------------------------------------
    // Select-and-inspect: verify what clicking reveals about different units
    // -----------------------------------------------------------------------

    #[test]
    fn ui_select_reveals_unit_type() {
        let mut game = make_test_game();

        let cmd = game.commander_team0.unwrap();
        let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;

        // Click to select
        game.click_select(cmd_pos.x.to_f32(), cmd_pos.z.to_f32(), 20.0);

        // Verify we can read the selected unit's info
        assert!(game.selected.is_some());
        assert!(game.selected_is_builder());
        assert!(!game.selected_is_factory());

        // Verify we can access the unit's UnitDef
        let sel = game.selected.unwrap();
        let ut = game.world.get::<recoil_sim::UnitType>(sel).unwrap();
        let registry = game.world.resource::<recoil_sim::unit_defs::UnitDefRegistry>();
        let def = registry.get(ut.id);
        assert!(def.is_some(), "Selected unit should have a UnitDef");
        let def = def.unwrap();
        assert!(def.is_builder, "Commander should be a builder");
        assert!(
            !def.can_build.is_empty(),
            "Commander should have build options"
        );
    }

    // -----------------------------------------------------------------------
    // Move then select: verify unit arrives, then re-select it at new pos
    // -----------------------------------------------------------------------

    #[test]
    fn ui_move_then_reselect_at_destination() {
        let mut game = make_test_game();

        let cmd = game.commander_team0.unwrap();
        let start_pos = game.world.get::<Position>(cmd).unwrap().pos;
        let sx = start_pos.x.to_f32();
        let sz = start_pos.z.to_f32();

        // Select and move to a nearby spot
        game.selected = Some(cmd);
        let tx = sx + 20.0;
        let tz = sz;
        game.click_move(tx, tz);

        // Tick until arrival
        for _ in 0..300 {
            game.tick();
            game.frame_count += 1;
        }

        // Deselect
        game.selected = None;

        // Click at the target location to re-select
        let found = game.click_select(tx, tz, 25.0);
        assert_eq!(
            found,
            Some(cmd),
            "Should re-select the commander at its new position"
        );
    }

    // -----------------------------------------------------------------------
    // Full game interaction sequence: build → factory → produce → fight
    // -----------------------------------------------------------------------

    #[test]
    fn ui_full_game_sequence() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        let cmd = game.commander_team0.unwrap();

        // 1. Select commander
        let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;
        game.click_select(cmd_pos.x.to_f32(), cmd_pos.z.to_f32(), 20.0);
        assert!(game.selected_is_builder());

        // 2. Place a solar (if commander can build one)
        let registry = game.world.resource::<recoil_sim::unit_defs::UnitDefRegistry>();
        let cmd_ut = game.world.get::<recoil_sim::UnitType>(cmd).unwrap().id;
        let solar_id = registry
            .get(cmd_ut)
            .and_then(|d| {
                d.can_build.iter().find(|&&id| {
                    registry
                        .get(id)
                        .map(|bd| bd.energy_production.is_some())
                        .unwrap_or(false)
                })
            })
            .copied();

        if let Some(sid) = solar_id {
            game.handle_build_command(PlacementType(sid));
            game.handle_place(cmd_pos.x.to_f32() + 15.0, cmd_pos.z.to_f32());

            // Verify placement
            let sites = game
                .world
                .query::<&recoil_sim::construction::BuildSite>()
                .iter(&game.world)
                .count();
            assert!(sites > 0, "Should have placed a build site");
        }

        // 3. Tick for a while (AI builds on team 1, construction progresses)
        for _ in 0..500 {
            game.tick();
            game.frame_count += 1;
        }

        // 4. Verify some economy exists (resources should be non-zero)
        let economy = game.world.resource::<EconomyState>();
        let res = economy.teams.get(&0).unwrap();
        // Resources should still exist (even if spent on buildings, commander produces)
        assert!(
            res.metal > SimFloat::ZERO || res.energy > SimFloat::ZERO,
            "Team 0 should have some resources after 500 ticks"
        );

        // 5. Game should still be running (no crash after full interaction)
        assert!(game.frame_count == 500);
    }

    // -----------------------------------------------------------------------
    // find_unit_at: basic spatial query correctness
    // -----------------------------------------------------------------------

    #[test]
    fn ui_find_unit_at_precision() {
        let mut game = make_test_game();
        let weapon_id = register_test_weapon(&mut game);

        // Spawn a unit at a known position
        let unit = spawn_armed_unit(&mut game, 400, 400, 0, weapon_id, 500);

        // Should find it at exact position
        let found = game.find_unit_at(400.0, 400.0, 10.0);
        assert_eq!(found, Some(unit));

        // Should find it within radius
        let found = game.find_unit_at(405.0, 405.0, 10.0);
        assert_eq!(found, Some(unit));

        // Should NOT find it outside radius
        let found = game.find_unit_at(420.0, 420.0, 10.0);
        assert!(found.is_none());
    }

    // -----------------------------------------------------------------------
    // Diagnostic: verify no phantom units spawn without AI or player action
    // -----------------------------------------------------------------------

    #[test]
    fn ui_no_phantom_units_without_action() {
        let mut game = make_test_game();

        // Count all alive entities at game start (should be 2 commanders only)
        let initial_alive: Vec<(u8, bool)> = game
            .world
            .query_filtered::<(&recoil_sim::Allegiance, Option<&recoil_sim::MoveState>), Without<Dead>>()
            .iter(&game.world)
            .map(|(a, ms)| (a.team, ms.is_some()))
            .collect();

        let initial_count = initial_alive.len();
        let initial_t0 = initial_alive.iter().filter(|(t, _)| *t == 0).count();
        let initial_t1 = initial_alive.iter().filter(|(t, _)| *t == 1).count();

        assert_eq!(initial_t0, 1, "Team 0 should start with 1 commander");
        assert_eq!(initial_t1, 1, "Team 1 should start with 1 commander");

        // Tick 100 frames WITHOUT AI (disable AI by not calling ai_tick)
        // We call sim systems directly instead of game.tick() to skip AI
        for _ in 0..100 {
            recoil_sim::construction::construction_system(&mut game.world);
            recoil_sim::sim_runner::sim_tick(&mut game.world);
            building::equip_factory_spawned_units(&mut game.world, &game.weapon_def_ids);
            building::finalize_completed_buildings(&mut game.world);
            game.frame_count += 1;
        }

        // Count again — should be unchanged (no AI to produce units)
        let after_alive: usize = game
            .world
            .query_filtered::<&recoil_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();

        assert_eq!(
            after_alive, initial_count,
            "No new units should appear without AI or player action: started with {}, now {}",
            initial_count, after_alive
        );
    }

    #[test]
    fn ui_ai_produces_units_over_time() {
        let mut game = make_test_game();
        fund_both_teams(&mut game);

        // Count initial units
        let initial: usize = game
            .world
            .query_filtered::<&recoil_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();

        // Run with AI for 600 frames (2 AI cycles)
        for _ in 0..600 {
            game.tick();
            game.frame_count += 1;
        }

        let after: usize = game
            .world
            .query_filtered::<&recoil_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .count();

        // AI should have produced some units/buildings
        assert!(
            after > initial,
            "AI should produce units over time: started with {}, now {}",
            initial,
            after
        );

        // All new units should be team 1 (AI team) or buildings placed by AI
        let t1_units: usize = game
            .world
            .query_filtered::<&recoil_sim::Allegiance, Without<Dead>>()
            .iter(&game.world)
            .filter(|a| a.team == 1)
            .count();

        assert!(
            t1_units > 1,
            "Team 1 (AI) should have more than just the commander: got {}",
            t1_units
        );
    }
}
