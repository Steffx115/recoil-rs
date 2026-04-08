//! Map manifest parsing and terrain grid setup.
//!
//! Supports two formats:
//! - **RON manifest** (`.ron` file): the engine's native format.
//! - **Spring SMF directory**: a directory containing `maps/*.smf` and
//!   optionally `mapinfo.lua` (BAR map format).

use std::path::Path;

use recoil_sim::map::{load_map_manifest, MapManifest};
use recoil_sim::smf::load_smf_map;

/// Loaded map data needed by the rest of game setup.
#[allow(dead_code)]
pub struct MapData {
    /// The raw manifest, if successfully loaded.
    pub manifest: Option<MapManifest>,
    /// Metal spot positions (x, z) extracted from the manifest.
    pub metal_spots: Vec<(f64, f64)>,
    /// Full simulation map data when loaded from SMF (carries heightmap,
    /// terrain grid, and features for downstream use).
    pub sim_map: Option<recoil_sim::map::MapData>,
}

/// Load and parse the map, returning extracted data.
///
/// If `map_path` is a directory, it is treated as a Spring SMF map.
/// Otherwise it is loaded as a RON manifest.
pub fn load_map(map_path: &Path) -> MapData {
    if map_path.is_dir() {
        load_smf_directory(map_path)
    } else {
        load_ron_manifest(map_path)
    }
}

fn load_smf_directory(map_dir: &Path) -> MapData {
    match load_smf_map(map_dir) {
        Ok(sim_map_data) => {
            let metal_spots = sim_map_data
                .manifest
                .metal_spots
                .iter()
                .map(|ms| (ms.x, ms.z))
                .collect();
            MapData {
                manifest: Some(sim_map_data.manifest.clone()),
                metal_spots,
                sim_map: Some(sim_map_data),
            }
        }
        Err(e) => {
            tracing::error!("Failed to load SMF map from {}: {e:#}", map_dir.display());
            MapData {
                manifest: None,
                metal_spots: Vec::new(),
                sim_map: None,
            }
        }
    }
}

fn load_ron_manifest(path: &Path) -> MapData {
    let manifest = load_map_manifest(path).ok();
    let metal_spots = if let Some(ref manifest) = manifest {
        tracing::info!(
            "Loaded map '{}' with {} start positions, {} metal spots",
            manifest.name,
            manifest.start_positions.len(),
            manifest.metal_spots.len(),
        );
        manifest.metal_spots.iter().map(|ms| (ms.x, ms.z)).collect()
    } else {
        Vec::new()
    };

    MapData {
        manifest,
        metal_spots,
        sim_map: None,
    }
}
