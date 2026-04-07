//! Game initialization: loading unit defs, map manifest, spawning commanders.

mod map_loader;
mod unit_loader;
mod world_init;

use std::collections::BTreeMap;
use std::path::Path;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use recoil_sim::sim_runner;

// Re-export public items so callers see the same API as before.
pub use unit_loader::load_unit_defs;
pub use world_init::spawn_commander;

/// Configuration produced by game setup, containing IDs needed later.
pub struct GameConfig {
    /// Weapon def IDs per unit type ID.
    pub weapon_def_ids: BTreeMap<u32, Vec<u32>>,
    /// Metal spot positions from the map manifest.
    pub metal_spots: Vec<(f64, f64)>,
    /// Commander entity for team 0.
    pub commander_team0: Option<Entity>,
    /// Commander entity for team 1.
    pub commander_team1: Option<Entity>,
}

/// Full game setup: init world, load defs, register weapons, spawn commanders.
///
/// Returns a `GameConfig` with entity handles and cached data.
pub fn setup_game(
    world: &mut World,
    bar_units_path: &Path,
    map_manifest_path: &Path,
) -> GameConfig {
    sim_runner::init_sim_world(world);

    let unit_def_registry = unit_loader::load_unit_defs(bar_units_path);
    let map_data = map_loader::load_map(map_manifest_path);

    world_init::init_world(world, unit_def_registry, &map_data)
}
