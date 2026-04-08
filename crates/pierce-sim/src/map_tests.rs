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
        type_map: None,
        type_map_width: 0,
        type_map_height: 0,
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
