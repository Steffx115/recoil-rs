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
mod tests {
    use super::*;

    /// Parameters for building a synthetic SMF binary.
    struct SyntheticSmf<'a> {
        mapx: i32,
        mapy: i32,
        square_size: i32,
        min_height: f32,
        max_height: f32,
        heightmap: &'a [u16],
        metal_map: &'a [u8],
        type_map: &'a [u8],
        feature_type_names: &'a [&'a str],
        features: &'a [RawFeature],
    }

    /// Build a minimal valid SMF binary with the given parameters.
    fn build_synthetic_smf(params: &SyntheticSmf) -> Vec<u8> {
        let SyntheticSmf {
            mapx,
            mapy,
            square_size,
            min_height,
            max_height,
            heightmap,
            metal_map,
            type_map,
            feature_type_names,
            features,
        } = params;
        let mut buf = Vec::new();

        // Header: 80 bytes.
        buf.extend_from_slice(SMF_MAGIC); // 0..16
        buf.extend_from_slice(&SMF_VERSION.to_le_bytes()); // 16..20
        buf.extend_from_slice(&0u32.to_le_bytes()); // 20..24: mapid
        buf.extend_from_slice(&mapx.to_le_bytes()); // 24..28
        buf.extend_from_slice(&mapy.to_le_bytes()); // 28..32
        buf.extend_from_slice(&square_size.to_le_bytes()); // 32..36
        buf.extend_from_slice(&8i32.to_le_bytes()); // 36..40: texelPerSquare
        buf.extend_from_slice(&32i32.to_le_bytes()); // 40..44: tilesize
        buf.extend_from_slice(&min_height.to_le_bytes()); // 44..48
        buf.extend_from_slice(&max_height.to_le_bytes()); // 48..52

        // Data offsets — we'll fill these in after we know the layout.
        let heightmap_ptr_offset = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // 52..56: heightmapPtr
        let type_map_ptr_offset = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // 56..60: typeMapPtr
        buf.extend_from_slice(&0u32.to_le_bytes()); // 60..64: tilesPtr
        buf.extend_from_slice(&0u32.to_le_bytes()); // 64..68: minimapPtr
        let metal_map_ptr_offset = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // 68..72: metalMapPtr
        let feature_ptr_offset = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // 72..76: featurePtr
        buf.extend_from_slice(&0i32.to_le_bytes()); // 76..80: numExtraHeaders
        assert_eq!(buf.len(), 80);

        // Heightmap data.
        let hm_offset = buf.len() as u32;
        for &v in *heightmap {
            buf.extend_from_slice(&v.to_le_bytes());
        }

        // Metal map data.
        let mm_offset = buf.len() as u32;
        buf.extend_from_slice(metal_map);

        // Type map data.
        let tm_offset = buf.len() as u32;
        buf.extend_from_slice(type_map);

        // Feature data.
        let feat_offset = buf.len() as u32;
        buf.extend_from_slice(&(feature_type_names.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(features.len() as i32).to_le_bytes());
        for name in *feature_type_names {
            buf.extend_from_slice(name.as_bytes());
            buf.push(0); // null terminator
        }
        for f in *features {
            buf.extend_from_slice(&f.feature_type_idx.to_le_bytes());
            buf.extend_from_slice(&f.xpos.to_le_bytes());
            buf.extend_from_slice(&f.ypos.to_le_bytes());
            buf.extend_from_slice(&f.zpos.to_le_bytes());
            buf.extend_from_slice(&f.rotation.to_le_bytes());
            buf.extend_from_slice(&f._relative_size.to_le_bytes());
        }

        // Patch pointers.
        buf[heightmap_ptr_offset..heightmap_ptr_offset + 4]
            .copy_from_slice(&hm_offset.to_le_bytes());
        buf[metal_map_ptr_offset..metal_map_ptr_offset + 4]
            .copy_from_slice(&mm_offset.to_le_bytes());
        buf[type_map_ptr_offset..type_map_ptr_offset + 4].copy_from_slice(&tm_offset.to_le_bytes());
        buf[feature_ptr_offset..feature_ptr_offset + 4].copy_from_slice(&feat_offset.to_le_bytes());

        buf
    }

    #[test]
    fn test_parse_smf_header() {
        let smf = build_synthetic_smf(&SyntheticSmf {
            mapx: 128,
            mapy: 128,
            square_size: 8,
            min_height: -100.0,
            max_height: 500.0,
            heightmap: &[0u16; 289], // (128/8+1)^2 = 17*17 = 289
            metal_map: &[0u8; 64],   // (128/16)^2 = 8*8 = 64
            type_map: &[0u8; 64],
            feature_type_names: &[],
            features: &[],
        });

        let header = parse_smf_header(&smf).unwrap();
        assert_eq!(header.mapx, 128);
        assert_eq!(header.mapy, 128);
        assert_eq!(header.square_size, 8);
        assert!((header.min_height - (-100.0)).abs() < f32::EPSILON);
        assert!((header.max_height - 500.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_smf_header_bad_magic() {
        let mut smf = vec![0u8; 80];
        smf[0..4].copy_from_slice(b"NOPE");
        assert!(parse_smf_header(&smf).is_err());
    }

    #[test]
    fn test_parse_smf_heightmap() {
        // 128x128 map, square_size=8 → heightmap is 17x17.
        let hm_count = 17 * 17;
        let mut heightmap = vec![100u16; hm_count];
        heightmap[0] = 500;
        heightmap[hm_count - 1] = 60000;

        let smf = build_synthetic_smf(&SyntheticSmf {
            mapx: 128,
            mapy: 128,
            square_size: 8,
            min_height: 0.0,
            max_height: 1000.0,
            heightmap: &heightmap,
            metal_map: &[0u8; 64],
            type_map: &[0u8; 64],
            feature_type_names: &[],
            features: &[],
        });

        let parsed = parse_smf(&smf).unwrap();
        assert_eq!(parsed.heightmap.len(), hm_count);
        assert_eq!(parsed.heightmap[0], 500);
        assert_eq!(parsed.heightmap[hm_count - 1], 60000);
    }

    #[test]
    fn test_parse_mapinfo_lua() {
        let lua = r#"
return {
    name = "Test Valley",
    teams = {
        [0] = { startPos = { x = 800.0, z = 1600.0 } },
        [1] = { startPos = { x = 6400.0, z = 6400.0 } },
    },
}
"#;
        let (name, positions) = parse_mapinfo_lua(lua).unwrap();
        assert_eq!(name, "Test Valley");
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].team, 0);
        assert!((positions[0].x - 800.0 / SPRING_ELMO_SCALE).abs() < 0.001);
        assert!((positions[0].z - 1600.0 / SPRING_ELMO_SCALE).abs() < 0.001);
        assert_eq!(positions[1].team, 1);
        assert!((positions[1].x - 6400.0 / SPRING_ELMO_SCALE).abs() < 0.001);
    }

    #[test]
    fn test_metal_map_clustering() {
        // 4x4 metal map with two clusters.
        #[rustfmt::skip]
        let metal_map: Vec<u8> = vec![
            255, 255,   0,   0,
            255,   0,   0,   0,
              0,   0,   0, 128,
              0,   0, 128, 128,
        ];

        let spots = cluster_metal_map(&metal_map, 4, 4, 8);
        assert_eq!(spots.len(), 2, "should find two clusters");

        // First cluster: top-left 3 cells (255+255+255).
        // Second cluster: bottom-right 3 cells (128+128+128).
        let total_metal: f64 = spots.iter().map(|s| s.metal_per_tick).sum();
        assert!(total_metal > 0.0);

        // Clusters should be in different positions.
        let d = (spots[0].x - spots[1].x).powi(2) + (spots[0].z - spots[1].z).powi(2);
        assert!(d > 0.0, "clusters should be at different positions");
    }

    #[test]
    fn test_smf_to_map_data_dimensions() {
        // mapx=128, mapy=128, square_size=8.
        // Heightmap: 17x17.  Metal/type map: 8x8.
        let hm_count = 17 * 17;
        let heightmap = vec![100u16; hm_count];
        let metal_map = vec![0u8; 64];
        let type_map = vec![1u8; 64];

        let smf = build_synthetic_smf(&SyntheticSmf {
            mapx: 128,
            mapy: 128,
            square_size: 8,
            min_height: 0.0,
            max_height: 1000.0,
            heightmap: &heightmap,
            metal_map: &metal_map,
            type_map: &type_map,
            feature_type_names: &[],
            features: &[],
        });
        let parsed = parse_smf(&smf).unwrap();

        let map_data = smf_to_map_data(parsed, "TestMap".to_string(), Vec::new()).unwrap();

        assert_eq!(map_data.manifest.name, "TestMap");
        assert_eq!(map_data.manifest.width, 17);
        assert_eq!(map_data.manifest.height, 17);
        assert!((map_data.manifest.cell_size - 1.0).abs() < 0.001);
        assert_eq!(map_data.heightmap.len(), hm_count);
        assert_eq!(map_data.terrain_grid.width(), 17);
        assert_eq!(map_data.terrain_grid.height(), 17);
        assert!(map_data.manifest.type_map.is_some());
        assert_eq!(map_data.manifest.type_map_width, 8);
        assert_eq!(map_data.manifest.type_map_height, 8);
    }

    #[test]
    fn test_smf_features_roundtrip() {
        let hm_count = 17 * 17;
        let features = vec![
            RawFeature {
                feature_type_idx: 0,
                xpos: 80.0,
                ypos: 10.0,
                zpos: 160.0,
                rotation: 1.5,
                _relative_size: 1.0,
            },
            RawFeature {
                feature_type_idx: 1,
                xpos: 320.0,
                ypos: 0.0,
                zpos: 640.0,
                rotation: 0.0,
                _relative_size: 0.5,
            },
        ];

        let hm = vec![100u16; hm_count];
        let smf = build_synthetic_smf(&SyntheticSmf {
            mapx: 128,
            mapy: 128,
            square_size: 8,
            min_height: 0.0,
            max_height: 1000.0,
            heightmap: &hm,
            metal_map: &[0u8; 64],
            type_map: &[0u8; 64],
            feature_type_names: &["TreeBirch", "RockGranite"],
            features: &features,
        });

        let parsed = parse_smf(&smf).unwrap();
        assert_eq!(parsed.feature_type_names.len(), 2);
        assert_eq!(parsed.feature_type_names[0], "TreeBirch");
        assert_eq!(parsed.feature_type_names[1], "RockGranite");
        assert_eq!(parsed.features.len(), 2);

        let map_data = smf_to_map_data(parsed, "FeatTest".to_string(), Vec::new()).unwrap();
        assert_eq!(map_data.features.len(), 2);
        assert_eq!(map_data.features[0].feature_type, "TreeBirch");
        assert!((map_data.features[0].x - 80.0 / SPRING_ELMO_SCALE).abs() < 0.001);
        assert_eq!(map_data.features[1].feature_type, "RockGranite");
    }

    #[test]
    fn test_load_smf_map_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let maps_dir = dir.path().join("maps");
        std::fs::create_dir(&maps_dir).unwrap();

        // Write synthetic SMF.
        let hm_count = 17 * 17;
        let hm = vec![0u16; hm_count];
        let smf = build_synthetic_smf(&SyntheticSmf {
            mapx: 128,
            mapy: 128,
            square_size: 8,
            min_height: 0.0,
            max_height: 1000.0,
            heightmap: &hm,
            metal_map: &[0u8; 64],
            type_map: &[0u8; 64],
            feature_type_names: &[],
            features: &[],
        });
        std::fs::write(maps_dir.join("test.smf"), &smf).unwrap();

        // Write mapinfo.lua.
        let mapinfo = r#"
return {
    name = "Dir Test Map",
    teams = {
        [0] = { startPos = { x = 100.0, z = 200.0 } },
    },
}
"#;
        std::fs::write(dir.path().join("mapinfo.lua"), mapinfo).unwrap();

        let map_data = load_smf_map(dir.path()).unwrap();
        assert_eq!(map_data.manifest.name, "Dir Test Map");
        assert_eq!(map_data.manifest.start_positions.len(), 1);
        assert!((map_data.manifest.start_positions[0].x - 100.0 / SPRING_ELMO_SCALE).abs() < 0.01);
    }
}
