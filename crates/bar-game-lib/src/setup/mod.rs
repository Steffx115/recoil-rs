//! Game initialization: loading unit defs, map manifest, spawning commanders.

mod map_loader;
mod unit_loader;
mod world_init;

use std::collections::BTreeMap;
use std::path::Path;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use pierce_sim::sim_runner;

// Re-export public items so callers see the same API as before.
pub use unit_loader::load_unit_defs;
pub use world_init::{spawn_commander, InitOptions};

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
    setup_game_with_options(world, bar_units_path, map_manifest_path, InitOptions::default())
}

pub fn setup_game_with_options(
    world: &mut World,
    bar_units_path: &Path,
    map_manifest_path: &Path,
    options: InitOptions,
) -> GameConfig {
    let unit_def_registry = unit_loader::load_unit_defs(bar_units_path);
    let map_data = map_loader::load_map(map_manifest_path);

    // Size grids to fit the map.
    // Map world extent = manifest.width * manifest.cell_size.
    // SpatialGrid cell_size=16 → spatial_w = world_extent / 16.
    // TerrainGrid cell_size=1  → terrain_w = world_extent.
    // Map cell_size is in world units per manifest cell.
    // TerrainGrid for pathfinding uses the manifest dimensions directly
    // (each terrain cell = one manifest cell = cell_size world units).
    let (spatial_w, spatial_h, terrain_w, terrain_h) = if let Some(ref manifest) = map_data.manifest {
        let world_w = manifest.width as usize * manifest.cell_size as usize;
        let world_h = manifest.height as usize * manifest.cell_size as usize;
        (
            (world_w / 16).max(64),
            (world_h / 16).max(64),
            manifest.width as usize,
            manifest.height as usize,
        )
    } else {
        (64, 64, 64, 64)
    };
    let terrain_cell_size = if let Some(ref manifest) = map_data.manifest {
        pierce_math::SimFloat::from_f64(manifest.cell_size)
    } else {
        pierce_math::SimFloat::from_int(16)
    };
    sim_runner::init_sim_world_sized(world, spatial_w, spatial_h, terrain_w, terrain_h, terrain_cell_size);

    world_init::init_world_with_options(world, unit_def_registry, &map_data, options)
}
