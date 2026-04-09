//! Game state management: tick loop, placement, factory queuing.

mod area_commands;
mod input_actions;

#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use pierce_math::SimFloat;
use pierce_sim::construction::Builder;
use pierce_sim::factory::BuildQueue;
use pierce_sim::projectile::ImpactEventQueue;
use pierce_sim::{Dead, Health, Position};

use pierce_sim::selection::SelectionState;

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
    pub selection: SelectionState,
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
    /// Cached sim capabilities (avoids per-tick TypeId lookups).
    sim_caps: pierce_sim::sim_runner::SimCapabilities,
}

impl GameState {
    /// Create a new game state, performing full game setup.
    pub fn new(bar_units_path: &Path, map_manifest_path: &Path) -> Self {
        Self::with_options(bar_units_path, map_manifest_path, setup::InitOptions::default())
    }

    /// Create a new game state with custom options (e.g. fog disabled).
    pub fn with_options(bar_units_path: &Path, map_manifest_path: &Path, options: setup::InitOptions) -> Self {
        let mut world = World::new();
        let config = setup::setup_game_with_options(&mut world, bar_units_path, map_manifest_path, options);

        let ai_state = AiState::new(42, 1, 0, config.commander_team1, config.commander_team0);
        let sim_caps = pierce_sim::sim_runner::SimCapabilities::detect(&world);

        Self {
            world,
            paused: false,
            frame_count: 0,
            selection: SelectionState::default(),
            placement_mode: None,
            ai_state,
            weapon_def_ids: config.weapon_def_ids,
            metal_spots: config.metal_spots,
            commander_team0: config.commander_team0,
            commander_team1: config.commander_team1,
            factory_team1: None,
            rng_seed: 12345,
            game_over: None,
            sim_caps,
        }
    }

    /// Create from an existing World and GameConfig (for testing or custom setups).
    pub fn from_config(world: World, config: GameConfig) -> Self {
        let ai_state = AiState::new(42, 1, 0, config.commander_team1, config.commander_team0);
        let sim_caps = pierce_sim::sim_runner::SimCapabilities::detect(&world);

        Self {
            world,
            paused: false,
            frame_count: 0,
            selection: SelectionState::default(),
            placement_mode: None,
            ai_state,
            weapon_def_ids: config.weapon_def_ids,
            metal_spots: config.metal_spots,
            commander_team0: config.commander_team0,
            commander_team1: config.commander_team1,
            factory_team1: None,
            rng_seed: 12345,
            game_over: None,
            sim_caps,
        }
    }

    /// Reset the game to a fresh state.
    pub fn reset(&mut self, bar_units_path: &Path, map_manifest_path: &Path) {
        self.world = World::new();
        let config = setup::setup_game(&mut self.world, bar_units_path, map_manifest_path);

        self.selection.clear();
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
        self.sim_caps = pierce_sim::sim_runner::SimCapabilities::detect(&self.world);
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

    /// Re-detect sim capabilities (call after inserting/removing resources like ComputeBackends).
    pub fn refresh_sim_caps(&mut self) {
        self.sim_caps = pierce_sim::sim_runner::SimCapabilities::detect(&self.world);
    }

    /// Run one simulation tick. Returns (impact_positions, death_positions) for rendering.
    pub fn tick(&mut self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        if self.game_over.is_some() || self.paused {
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
        pierce_sim::construction::construction_system(&mut self.world);

        // Run all systems via sim_runner
        pierce_sim::sim_runner::sim_tick_with(&mut self.world, &self.sim_caps);

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

    /// The first selected entity (backwards-compat convenience).
    pub fn selected(&self) -> Option<Entity> {
        self.selection.selected.first().copied()
    }

    /// Check if the (first) selected entity is a factory.
    pub fn selected_is_factory(&self) -> bool {
        self.selected()
            .filter(|&e| self.world.get_entity(e).is_ok())
            .map(|e| self.world.get::<BuildQueue>(e).is_some())
            .unwrap_or(false)
    }

    /// Check if the (first) selected entity is a commander/builder.
    pub fn selected_is_builder(&self) -> bool {
        self.selected()
            .filter(|&e| self.world.get_entity(e).is_ok())
            .map(|e| self.world.get::<Builder>(e).is_some())
            .unwrap_or(false)
    }

    /// Queue a unit in a factory by name.
    pub fn handle_factory_queue(&mut self, factory_entity: Entity, unit_name: &str) {
        production::queue_unit_by_name(&mut self.world, factory_entity, unit_name);
    }

    /// Queue a unit in a factory by type ID (data-driven).
    pub fn queue_unit_in_factory(&mut self, factory_entity: Entity, unit_type_id: u32) {
        production::queue_unit(&mut self.world, factory_entity, unit_type_id);
    }
}
