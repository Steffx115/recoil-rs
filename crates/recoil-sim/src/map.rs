//! Custom map format and loader for the Recoil RTS engine.
//!
//! Maps are defined by a [`MapManifest`] (serialized as RON) plus a heightmap
//! (`Vec<u16>`, row-major). At runtime these are combined into [`MapData`]
//! which also holds the derived [`TerrainGrid`] for pathfinding.

use std::path::Path;

use serde::{Deserialize, Serialize};

use bevy_ecs::system::Resource;
use recoil_math::SimFloat;

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
    };

    let terrain_grid = heightmap_to_terrain_grid(&heightmap, width, height);

    MapData {
        manifest,
        heightmap,
        terrain_grid,
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
mod tests {
    use super::*;

    #[test]
    fn generate_flat_map_dimensions() {
        let map = generate_flat_map("test", 64, 64, 1.0, 4);
        assert_eq!(map.heightmap.len(), 64 * 64);
        assert_eq!(map.terrain_grid.width(), 64);
        assert_eq!(map.terrain_grid.height(), 64);
        assert_eq!(map.manifest.width, 64);
        assert_eq!(map.manifest.height, 64);
    }

    #[test]
    fn generate_flat_map_start_positions() {
        let map = generate_flat_map("test", 64, 64, 1.0, 4);
        assert_eq!(map.manifest.start_positions.len(), 4);
        for (i, sp) in map.manifest.start_positions.iter().enumerate() {
            assert_eq!(sp.team, i as u8);
        }
    }

    #[test]
    fn generate_flat_map_metal_spots() {
        let map = generate_flat_map("test", 64, 64, 1.0, 2);
        assert!(
            !map.manifest.metal_spots.is_empty(),
            "flat map should have metal spots"
        );
    }

    #[test]
    fn heightmap_flat_cost_one() {
        let heightmap = vec![100u16; 16]; // 4x4, uniform
        let grid = heightmap_to_terrain_grid(&heightmap, 4, 4);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    grid.get(x, y).unwrap(),
                    SimFloat::ONE,
                    "flat terrain should have cost 1 at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn heightmap_steep_slope_higher_cost() {
        // 3x1 strip: [0, 500, 0] — moderate slope at centre.
        let heightmap = vec![0u16, 500, 0];
        let grid = heightmap_to_terrain_grid(&heightmap, 3, 1);

        // Centre cell should have cost > 1 (slope delta = 500).
        let centre_cost = grid.get(1, 0).unwrap();
        assert!(
            centre_cost > SimFloat::ONE,
            "steep slope should have cost > 1, got {centre_cost:?}"
        );
    }

    #[test]
    fn heightmap_cliff_impassable() {
        // 3x1 strip with a cliff in the middle.
        let heightmap = vec![0u16, IMPASSABLE_SLOPE_THRESHOLD + 1000, 0];
        let grid = heightmap_to_terrain_grid(&heightmap, 3, 1);

        let centre_cost = grid.get(1, 0).unwrap();
        assert_eq!(
            centre_cost,
            SimFloat::ZERO,
            "cliff should be impassable (cost 0)"
        );
    }

    #[test]
    fn ron_roundtrip() {
        let manifest = MapManifest {
            name: "roundtrip_test".to_string(),
            width: 32,
            height: 32,
            cell_size: 2.0,
            water_level: 5.0,
            start_positions: vec![StartPosition {
                x: 10.0,
                z: 20.0,
                team: 0,
            }],
            metal_spots: vec![MetalSpot {
                x: 5.0,
                z: 15.0,
                metal_per_tick: 0.5,
            }],
        };

        let dir = std::env::temp_dir();
        let path = dir.join("recoil_map_test.ron");

        save_map_manifest(&manifest, &path).expect("save should succeed");
        let loaded = load_map_manifest(&path).expect("load should succeed");

        assert_eq!(loaded.name, manifest.name);
        assert_eq!(loaded.width, manifest.width);
        assert_eq!(loaded.height, manifest.height);
        assert!((loaded.cell_size - manifest.cell_size).abs() < 1e-9);
        assert!((loaded.water_level - manifest.water_level).abs() < 1e-9);
        assert_eq!(loaded.start_positions.len(), 1);
        assert_eq!(loaded.metal_spots.len(), 1);
        assert!((loaded.metal_spots[0].metal_per_tick - 0.5).abs() < 1e-9);

        // Cleanup.
        let _ = std::fs::remove_file(&path);
    }
}
