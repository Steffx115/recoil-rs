//! Spring Map Format (SMF) loader.
//!
//! Parses `.smf` binary files and `mapinfo.lua` to produce a [`MapData`]
//! compatible with the Recoil engine.  Coordinate conversion from Spring
//! elmos to Recoil world units is applied automatically.

use std::collections::VecDeque;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::lua_unitdefs::parse_lua_table;
use crate::map::{
    heightmap_to_terrain_grid, FeaturePlacement, MapData, MapManifest, MetalSpot, StartPosition,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SMF_MAGIC: &[u8; 16] = b"spring map file\0";
const SMF_VERSION: i32 = 1;

/// Spring coordinates → Recoil world coords.
const SPRING_ELMO_SCALE: f64 = 8.0;

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Parsed SMF file header (80 bytes).
#[derive(Debug)]
#[allow(dead_code)]
struct SmfHeader {
    mapx: i32,
    mapy: i32,
    square_size: i32,
    min_height: f32,
    max_height: f32,
    heightmap_ptr: u32,
    type_map_ptr: u32,
    metal_map_ptr: u32,
    feature_ptr: u32,
}

/// A raw feature record from the SMF binary.
#[derive(Debug)]
struct RawFeature {
    feature_type_idx: i32,
    xpos: f32,
    ypos: f32,
    zpos: f32,
    rotation: f32,
    _relative_size: f32,
}

/// Everything extracted from the binary SMF file.
struct SmfParsed {
    header: SmfHeader,
    heightmap: Vec<u16>,
    metal_map: Vec<u8>,
    metal_map_width: u32,
    metal_map_height: u32,
    type_map: Vec<u8>,
    type_map_width: u32,
    type_map_height: u32,
    feature_type_names: Vec<String>,
    features: Vec<RawFeature>,
}

// ---------------------------------------------------------------------------
// Binary helpers
// ---------------------------------------------------------------------------

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// SMF binary parsing
// ---------------------------------------------------------------------------

fn parse_smf_header(data: &[u8]) -> Result<SmfHeader> {
    if data.len() < 80 {
        bail!(
            "SMF file too small ({} bytes, need at least 80)",
            data.len()
        );
    }
    if &data[0..16] != SMF_MAGIC {
        bail!("Invalid SMF magic bytes");
    }
    let version = read_i32(data, 16);
    if version != SMF_VERSION {
        bail!("Unsupported SMF version {version}, expected {SMF_VERSION}");
    }

    Ok(SmfHeader {
        mapx: read_i32(data, 24),
        mapy: read_i32(data, 28),
        square_size: read_i32(data, 32),
        min_height: read_f32(data, 44),
        max_height: read_f32(data, 48),
        heightmap_ptr: read_u32(data, 52),
        type_map_ptr: read_u32(data, 56),
        metal_map_ptr: read_u32(data, 68),
        feature_ptr: read_u32(data, 72),
    })
}

fn parse_smf_heightmap(data: &[u8], header: &SmfHeader) -> Result<Vec<u16>> {
    let sq = header.square_size as u32;
    let w = (header.mapx as u32) / sq + 1;
    let h = (header.mapy as u32) / sq + 1;
    let count = (w * h) as usize;
    let start = header.heightmap_ptr as usize;
    let end = start + count * 2;
    if end > data.len() {
        bail!(
            "SMF heightmap extends past end of file (need {end}, have {})",
            data.len()
        );
    }
    let mut heightmap = Vec::with_capacity(count);
    for i in 0..count {
        heightmap.push(read_u16(data, start + i * 2));
    }
    Ok(heightmap)
}

fn parse_smf_metal_map(data: &[u8], header: &SmfHeader) -> Result<(Vec<u8>, u32, u32)> {
    let sq = header.square_size as u32;
    let w = (header.mapx as u32) / (sq * 2);
    let h = (header.mapy as u32) / (sq * 2);
    let count = (w * h) as usize;
    let start = header.metal_map_ptr as usize;
    let end = start + count;
    if end > data.len() {
        bail!("SMF metal map extends past end of file");
    }
    Ok((data[start..end].to_vec(), w, h))
}

fn parse_smf_type_map(data: &[u8], header: &SmfHeader) -> Result<(Vec<u8>, u32, u32)> {
    let sq = header.square_size as u32;
    let w = (header.mapx as u32) / (sq * 2);
    let h = (header.mapy as u32) / (sq * 2);
    let count = (w * h) as usize;
    let start = header.type_map_ptr as usize;
    let end = start + count;
    if end > data.len() {
        bail!("SMF type map extends past end of file");
    }
    Ok((data[start..end].to_vec(), w, h))
}

fn parse_smf_features(data: &[u8], header: &SmfHeader) -> Result<(Vec<String>, Vec<RawFeature>)> {
    let start = header.feature_ptr as usize;
    if start == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    if start + 8 > data.len() {
        bail!("SMF feature header extends past end of file");
    }

    let num_types = read_i32(data, start) as usize;
    let num_features = read_i32(data, start + 4) as usize;
    let mut offset = start + 8;

    // Read null-terminated feature type name strings.
    let mut type_names = Vec::with_capacity(num_types);
    for _ in 0..num_types {
        let mut name = String::new();
        while offset < data.len() && data[offset] != 0 {
            name.push(data[offset] as char);
            offset += 1;
        }
        if offset < data.len() {
            offset += 1; // skip null terminator
        }
        type_names.push(name);
    }

    // Read feature records (24 bytes each).
    let mut features = Vec::with_capacity(num_features);
    for _ in 0..num_features {
        if offset + 24 > data.len() {
            break;
        }
        features.push(RawFeature {
            feature_type_idx: read_i32(data, offset),
            xpos: read_f32(data, offset + 4),
            ypos: read_f32(data, offset + 8),
            zpos: read_f32(data, offset + 12),
            rotation: read_f32(data, offset + 16),
            _relative_size: read_f32(data, offset + 20),
        });
        offset += 24;
    }

    Ok((type_names, features))
}

fn parse_smf(data: &[u8]) -> Result<SmfParsed> {
    let header = parse_smf_header(data)?;
    let heightmap = parse_smf_heightmap(data, &header)?;
    let (metal_map, metal_map_width, metal_map_height) = parse_smf_metal_map(data, &header)?;
    let (type_map, type_map_width, type_map_height) = parse_smf_type_map(data, &header)?;
    let (feature_type_names, features) = parse_smf_features(data, &header)?;

    Ok(SmfParsed {
        header,
        heightmap,
        metal_map,
        metal_map_width,
        metal_map_height,
        type_map,
        type_map_width,
        type_map_height,
        feature_type_names,
        features,
    })
}

// ---------------------------------------------------------------------------
// mapinfo.lua parsing
// ---------------------------------------------------------------------------

/// Parse a `mapinfo.lua` file for the map name and start positions.
fn parse_mapinfo_lua(source: &str) -> Result<(String, Vec<StartPosition>)> {
    let table = parse_lua_table(source)?;

    let name = table
        .values
        .get("name")
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());

    let mut start_positions = Vec::new();

    if let Some(teams) = table.sub_tables.get("teams") {
        for (&team_idx, team_table) in &teams.array_entries {
            if let Some(sp) = team_table
                .sub_tables
                .get("startPos")
                .or_else(|| team_table.sub_tables.get("startpos"))
            {
                let x = sp
                    .values
                    .get("x")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let z = sp
                    .values
                    .get("z")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                start_positions.push(StartPosition {
                    x: x / SPRING_ELMO_SCALE,
                    z: z / SPRING_ELMO_SCALE,
                    team: team_idx as u8,
                });
            }
        }
    }

    // Sort by team index for determinism.
    start_positions.sort_by_key(|sp| sp.team);

    Ok((name, start_positions))
}

// ---------------------------------------------------------------------------
// Metal map clustering
// ---------------------------------------------------------------------------

/// Extract metal spots from a raw metal map using flood-fill clustering.
///
/// Adjacent non-zero cells are grouped into clusters.  Each cluster produces
/// one [`MetalSpot`] at its weighted centroid.
fn cluster_metal_map(
    metal_map: &[u8],
    width: u32,
    height: u32,
    square_size: i32,
) -> Vec<MetalSpot> {
    let w = width as usize;
    let h = height as usize;
    let mut visited = vec![false; w * h];
    let mut spots = Vec::new();

    // Each metal map cell covers (square_size * 2) Spring elmos.
    let cell_elmos = (square_size * 2) as f64;

    for start_y in 0..h {
        for start_x in 0..w {
            let idx = start_y * w + start_x;
            if visited[idx] || metal_map[idx] == 0 {
                continue;
            }

            // BFS flood-fill.
            let mut queue = VecDeque::new();
            queue.push_back((start_x, start_y));
            visited[idx] = true;

            let mut sum_x: f64 = 0.0;
            let mut sum_z: f64 = 0.0;
            let mut total_weight: f64 = 0.0;

            while let Some((cx, cy)) = queue.pop_front() {
                let val = metal_map[cy * w + cx] as f64;
                // Center of this cell in Spring elmos.
                let ex = (cx as f64 + 0.5) * cell_elmos;
                let ez = (cy as f64 + 0.5) * cell_elmos;
                sum_x += ex * val;
                sum_z += ez * val;
                total_weight += val;

                // 4-connected neighbours.
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                ] {
                    if nx < w && ny < h {
                        let ni = ny * w + nx;
                        if !visited[ni] && metal_map[ni] > 0 {
                            visited[ni] = true;
                            queue.push_back((nx, ny));
                        }
                    }
                }
            }

            if total_weight > 0.0 {
                spots.push(MetalSpot {
                    x: (sum_x / total_weight) / SPRING_ELMO_SCALE,
                    z: (sum_z / total_weight) / SPRING_ELMO_SCALE,
                    metal_per_tick: total_weight / 255.0,
                });
            }
        }
    }

    spots
}

// ---------------------------------------------------------------------------
// Conversion to Recoil MapData
// ---------------------------------------------------------------------------

fn smf_to_map_data(
    parsed: SmfParsed,
    map_name: String,
    start_positions: Vec<StartPosition>,
) -> Result<MapData> {
    let sq = parsed.header.square_size as u32;
    let hm_w = (parsed.header.mapx as u32) / sq + 1;
    let hm_h = (parsed.header.mapy as u32) / sq + 1;

    // The heightmap dimensions in Recoil cells.  Each SMF heightmap sample
    // corresponds to one cell of size (square_size / SPRING_ELMO_SCALE) world
    // units.
    let cell_size = parsed.header.square_size as f64 / SPRING_ELMO_SCALE;

    // Metal spots from clustering.
    let metal_spots = cluster_metal_map(
        &parsed.metal_map,
        parsed.metal_map_width,
        parsed.metal_map_height,
        parsed.header.square_size,
    );

    // Features.
    let features: Vec<FeaturePlacement> = parsed
        .features
        .iter()
        .map(|f| {
            let type_name = if (f.feature_type_idx as usize) < parsed.feature_type_names.len() {
                parsed.feature_type_names[f.feature_type_idx as usize].clone()
            } else {
                format!("unknown_{}", f.feature_type_idx)
            };
            FeaturePlacement {
                feature_type: type_name,
                x: f.xpos as f64 / SPRING_ELMO_SCALE,
                y: f.ypos as f64 / SPRING_ELMO_SCALE,
                z: f.zpos as f64 / SPRING_ELMO_SCALE,
                rotation: f.rotation as f64,
            }
        })
        .collect();

    // Water level: Spring stores min_height / max_height as the range mapped
    // to u16 0..65535.  Water level 0 in Spring corresponds to the midpoint.
    // For simplicity, store the scaled min_height as water level reference.
    let water_level = parsed.header.min_height as f64 / SPRING_ELMO_SCALE;

    let manifest = MapManifest {
        name: map_name,
        width: hm_w,
        height: hm_h,
        cell_size,
        water_level,
        start_positions,
        metal_spots,
        type_map: Some(parsed.type_map),
        type_map_width: parsed.type_map_width,
        type_map_height: parsed.type_map_height,
    };

    let terrain_grid = heightmap_to_terrain_grid(&parsed.heightmap, hm_w, hm_h);

    Ok(MapData {
        manifest,
        heightmap: parsed.heightmap,
        terrain_grid,
        features,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load a BAR map from a directory containing `maps/*.smf` and `mapinfo.lua`.
///
/// The directory layout should be:
/// ```text
/// map_dir/
///   maps/
///     SomeName.smf
///   mapinfo.lua
/// ```
pub fn load_smf_map(map_dir: &Path) -> Result<MapData> {
    // Find the .smf file.
    let maps_dir = map_dir.join("maps");
    let smf_path = if maps_dir.is_dir() {
        find_smf_in_dir(&maps_dir)?
    } else {
        // Maybe the smf is directly in map_dir.
        find_smf_in_dir(map_dir)?
    };

    let smf_data = std::fs::read(&smf_path)
        .with_context(|| format!("Failed to read {}", smf_path.display()))?;
    let parsed = parse_smf(&smf_data)
        .with_context(|| format!("Failed to parse SMF: {}", smf_path.display()))?;

    // Parse mapinfo.lua for start positions and map name.
    let mapinfo_path = map_dir.join("mapinfo.lua");
    let (map_name, start_positions) = if mapinfo_path.exists() {
        let source = std::fs::read_to_string(&mapinfo_path)
            .with_context(|| format!("Failed to read {}", mapinfo_path.display()))?;
        parse_mapinfo_lua(&source)
            .with_context(|| format!("Failed to parse {}", mapinfo_path.display()))?
    } else {
        // Derive name from directory.
        let name = map_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();
        (name, Vec::new())
    };

    tracing::info!(
        "Loaded SMF map '{}' ({}x{}, {} metal spots, {} features)",
        map_name,
        parsed.header.mapx,
        parsed.header.mapy,
        parsed.metal_map.iter().filter(|&&v| v > 0).count(),
        parsed.features.len(),
    );

    smf_to_map_data(parsed, map_name, start_positions)
}

/// Find the first `.smf` file in a directory.
fn find_smf_in_dir(dir: &Path) -> Result<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("smf") {
            return Ok(path);
        }
    }
    bail!("No .smf file found in {}", dir.display())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "smf_tests.rs"]
mod tests;
