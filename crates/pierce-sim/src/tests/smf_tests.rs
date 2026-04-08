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
