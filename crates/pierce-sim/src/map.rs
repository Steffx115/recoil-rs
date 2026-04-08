//! Custom map format and loader for the Pierce RTS engine.
//!
//! Maps are defined by a [`MapManifest`] (serialized as RON) plus a heightmap
//! (`Vec<u16>`, row-major). At runtime these are combined into [`MapData`]
//! which also holds the derived [`TerrainGrid`] for pathfinding.

use std::path::Path;

use serde::{Deserialize, Serialize};

use bevy_ecs::system::Resource;
use pierce_math::SimFloat;

use crate::pathfinding::TerrainGrid;

// ---------------------------------------------------------------------------
// Manifest types (RON-serializable)
// ---------------------------------------------------------------------------

/// Top-level map descriptor, stored as a `.ron` file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MapManifest {
    pub name: String,
    /// Map width in cells.
    pub width: u32,
    /// Map height in cells.
    pub height: u32,
    /// World units per cell.
    pub cell_size: f64,
    /// Water level (height values below this are underwater).
    pub water_level: f64,
    /// Per-team spawn locations.
    pub start_positions: Vec<StartPosition>,
    /// Extractable metal deposits.
    pub metal_spots: Vec<MetalSpot>,
    /// Terrain type map (grass/rock/water), if loaded from SMF.
    #[serde(default)]
    pub type_map: Option<Vec<u8>>,
    /// Width of the type map grid.
    #[serde(default)]
    pub type_map_width: u32,
    /// Height of the type map grid.
    #[serde(default)]
    pub type_map_height: u32,
}

/// A team's starting position on the map.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StartPosition {
    pub x: f64,
    pub z: f64,
    pub team: u8,
}

/// A metal extraction point on the map.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetalSpot {
    pub x: f64,
    pub z: f64,
    pub metal_per_tick: f64,
}

/// A pre-placed map feature (tree, rock, wreckage, etc.).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FeaturePlacement {
    pub feature_type: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub rotation: f64,
}

/// ECS resource holding all metal spots on the current map.
#[derive(Resource, Debug, Clone, Default)]
pub struct MetalSpots {
    pub spots: Vec<MetalSpot>,
}

impl MetalSpots {
    pub fn nearest(&self, x: f64, z: f64, max_distance: f64) -> Option<&MetalSpot> {
        let max_dist_sq = max_distance * max_distance;
        self.spots
            .iter()
            .map(|s| {
                let dx = s.x - x;
                let dz = s.z - z;
                (s, dx * dx + dz * dz)
            })
            .filter(|(_, d)| *d <= max_dist_sq)
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(s, _)| s)
    }
}

// ---------------------------------------------------------------------------
// Runtime map data
// ---------------------------------------------------------------------------

/// Fully loaded map ready for simulation use.
pub struct MapData {
    pub manifest: MapManifest,
    /// Row-major heightmap, `width * height` entries.
    pub heightmap: Vec<u16>,
    /// Pathfinding cost grid derived from the heightmap.
    pub terrain_grid: TerrainGrid,
    /// Pre-placed features from the map (trees, rocks, etc.).
    pub features: Vec<FeaturePlacement>,
}

// ---------------------------------------------------------------------------
// Heightmap → TerrainGrid conversion
// ---------------------------------------------------------------------------

/// Maximum height difference between adjacent cells before the cell is
/// considered impassable (cliff).
const IMPASSABLE_SLOPE_THRESHOLD: u16 = 8000;

/// Convert a row-major `u16` heightmap into a [`TerrainGrid`].
///
/// The algorithm inspects the maximum height difference between each cell and
/// its four cardinal neighbours:
/// - Flat (delta == 0): cost 1
/// - Moderate slope: cost scales linearly with delta
/// - Very steep (delta >= [`IMPASSABLE_SLOPE_THRESHOLD`]): impassable (cost 0)
pub fn heightmap_to_terrain_grid(heightmap: &[u16], width: u32, height: u32) -> TerrainGrid {
    let w = width as usize;
    let h = height as usize;
    assert_eq!(
        heightmap.len(),
        w * h,
        "heightmap length must equal width * height"
    );

    let mut grid = TerrainGrid::new(w, h, SimFloat::ONE);

    for y in 0..h {
        for x in 0..w {
            let center = heightmap[y * w + x];
            let mut max_delta: u16 = 0;

            // Cardinal neighbours.
            if x > 0 {
                max_delta = max_delta.max(center.abs_diff(heightmap[y * w + (x - 1)]));
            }
            if x + 1 < w {
                max_delta = max_delta.max(center.abs_diff(heightmap[y * w + (x + 1)]));
            }
            if y > 0 {
                max_delta = max_delta.max(center.abs_diff(heightmap[(y - 1) * w + x]));
            }
            if y + 1 < h {
                max_delta = max_delta.max(center.abs_diff(heightmap[(y + 1) * w + x]));
            }

            if max_delta >= IMPASSABLE_SLOPE_THRESHOLD {
                grid.set(x, y, SimFloat::ZERO); // impassable
            } else if max_delta > 0 {
                // Linear cost scaling: cost = 1 + delta / 1000.
                // Use from_ratio to stay in SimFloat domain.
                let extra = SimFloat::from_ratio(max_delta as i32, 1000);
                grid.set(x, y, SimFloat::ONE + extra);
            }
            // else: flat, keep default cost of ONE.
        }
    }

    grid
}

// ---------------------------------------------------------------------------
// Map generation
// ---------------------------------------------------------------------------

/// Generate a flat map with evenly-spaced start positions and pseudo-random
/// metal spots.
///
/// The heightmap is uniform (all zeros) so every cell has cost 1. Start
/// positions are distributed evenly along the map edges. Metal spots are
/// placed using a simple deterministic pattern (not true RNG, to keep the
/// function pure and deterministic).
pub fn generate_flat_map(
    name: &str,
    width: u32,
    height: u32,
    cell_size: f64,
    num_players: u8,
) -> MapData {
    let total_cells = (width as usize) * (height as usize);
    let heightmap = vec![0u16; total_cells];

    // Evenly spaced start positions around the map perimeter.
    let mut start_positions = Vec::with_capacity(num_players as usize);
    for i in 0..num_players {
        let angle = (i as f64) * std::f64::consts::TAU / (num_players as f64);
        let cx = (width as f64) * cell_size / 2.0;
        let cz = (height as f64) * cell_size / 2.0;
        let radius = cx.min(cz) * 0.8;
        start_positions.push(StartPosition {
            x: cx + radius * angle.cos(),
            z: cz + radius * angle.sin(),
            team: i,
        });
    }

    // Deterministic metal spots: a grid of spots in the interior.
    let metal_spacing = 8u32;
    let mut metal_spots = Vec::new();
    let mut y = metal_spacing;
    while y < height.saturating_sub(metal_spacing) {
        let mut x = metal_spacing;
        while x < width.saturating_sub(metal_spacing) {
            metal_spots.push(MetalSpot {
                x: (x as f64) * cell_size,
                z: (y as f64) * cell_size,
                metal_per_tick: 1.0,
            });
            x += metal_spacing;
        }
        y += metal_spacing;
    }

    let manifest = MapManifest {
        name: name.to_string(),
        width,
        height,
        cell_size,
        water_level: 0.0,
        start_positions,
        metal_spots,
        type_map: None,
        type_map_width: 0,
        type_map_height: 0,
    };

    let terrain_grid = heightmap_to_terrain_grid(&heightmap, width, height);

    MapData {
        manifest,
        heightmap,
        terrain_grid,
        features: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// RON I/O
// ---------------------------------------------------------------------------

/// Load a [`MapManifest`] from a RON file.
pub fn load_map_manifest(path: &Path) -> anyhow::Result<MapManifest> {
    let contents = std::fs::read_to_string(path)?;
    let manifest: MapManifest = ron::from_str(&contents)?;
    Ok(manifest)
}

/// Save a [`MapManifest`] to a RON file.
pub fn save_map_manifest(manifest: &MapManifest, path: &Path) -> anyhow::Result<()> {
    let pretty = ron::ser::PrettyConfig::default();
    let s = ron::ser::to_string_pretty(manifest, pretty)?;
    std::fs::write(path, s)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/map_tests.rs"]
mod tests;
