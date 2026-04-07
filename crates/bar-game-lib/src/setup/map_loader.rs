//! Map manifest parsing and terrain grid setup.

use std::path::Path;

use recoil_sim::map::{load_map_manifest, MapManifest};

/// Loaded map data needed by the rest of game setup.
pub struct MapData {
    /// The raw manifest, if successfully loaded.
    pub manifest: Option<MapManifest>,
    /// Metal spot positions (x, z) extracted from the manifest.
    pub metal_spots: Vec<(f64, f64)>,
}

/// Load and parse the map manifest, returning extracted data.
pub fn load_map(map_manifest_path: &Path) -> MapData {
    let manifest = load_map_manifest(map_manifest_path).ok();
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
    }
}
