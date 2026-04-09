use super::*;
use pierce_sim::economy::init_economy;
use pierce_sim::sim_runner;

fn setup_world_with_economy() -> World {
    let mut world = World::new();
    sim_runner::init_sim_world(&mut world);
    // Override the default 64x64 grid with a larger one so test
    // coordinates (up to ~200) are within bounds.
    world.insert_resource(pierce_sim::pathfinding::TerrainGrid::new(
        256,
        256,
        SimFloat::ONE,
    ));
    init_economy(&mut world, &[0, 1]);
    // Give team 0 plenty of resources for tests
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&0) {
            res.metal = SimFloat::from_int(10000);
            res.energy = SimFloat::from_int(50000);
        }
    }
    // Insert a minimal UnitDefRegistry with a solar building def
    let mut registry = UnitDefRegistry::default();
    registry.register(pierce_sim::unit_defs::UnitDef {
        name: "testsolar".into(),
        unit_type_id: BUILDING_SOLAR_ID,
        max_health: 500.0,
        armor_class: "Building".into(),
        sight_range: 200.0,
        collision_radius: 32.0,
        max_speed: 0.0,
        acceleration: 0.0,
        turn_rate: 0.0,
        metal_cost: 150.0,
        energy_cost: 0.0,
        build_time: 120,
        weapons: vec![],
        model_path: None,
        icon_path: None,
        categories: vec![],
        can_build: vec![],
        can_build_names: vec![],
        build_power: None,
        metal_production: None,
        energy_production: Some(20.0),
        is_building: true,
        is_builder: false,
    });
    world.insert_resource(registry);
    world
}

/// Assert the terrain grid covers a building at `(x, z)` with `radius`.
fn assert_world_grid_covers(world: &World, x: f32, z: f32, radius: f32) {
    let grid = world.resource::<TerrainGrid>();
    let max_x = (x + radius).ceil() as usize;
    let max_z = (z + radius).ceil() as usize;
    assert!(
        max_x < grid.width() && max_z < grid.height(),
        "Terrain grid {}x{} too small for ({x}, {z}) r={radius} (needs {}x{})",
        grid.width(),
        grid.height(),
        max_x + 1,
        max_z + 1,
    );
}

#[test]
fn test_place_building_solar() {
    let mut world = setup_world_with_economy();
    assert_world_grid_covers(&world, 100.0, 100.0, 32.0);
    let result = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(result.is_some());
    let entity = result.unwrap();
    assert!(world.get::<BuildSite>(entity).is_some());
    assert_eq!(world.get::<UnitType>(entity).unwrap().id, BUILDING_SOLAR_ID);
}

#[test]
fn test_place_building_cannot_afford() {
    let mut world = World::new();
    sim_runner::init_sim_world(&mut world);
    init_economy(&mut world, &[0, 1]);
    // Drain resources
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&0) {
            res.metal = SimFloat::ZERO;
            res.energy = SimFloat::ZERO;
        }
    }
    // Insert a minimal registry
    let mut registry = UnitDefRegistry::default();
    registry.register(pierce_sim::unit_defs::UnitDef {
        name: "testsolar".into(),
        unit_type_id: BUILDING_SOLAR_ID,
        max_health: 500.0,
        armor_class: "Building".into(),
        sight_range: 200.0,
        collision_radius: 32.0,
        max_speed: 0.0,
        acceleration: 0.0,
        turn_rate: 0.0,
        metal_cost: 150.0,
        energy_cost: 0.0,
        build_time: 120,
        weapons: vec![],
        model_path: None,
        icon_path: None,
        categories: vec![],
        can_build: vec![],
        can_build_names: vec![],
        build_power: None,
        metal_production: None,
        energy_production: Some(20.0),
        is_building: true,
        is_builder: false,
    });
    world.insert_resource(registry);

    let result = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(result.is_none());
}

#[test]
fn test_place_building_blocked_by_existing_building() {
    let mut world = setup_world_with_economy();
    assert_world_grid_covers(&world, 100.0, 100.0, 32.0);

    // Place first building — should succeed.
    let first = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(first.is_some(), "first building should succeed");

    // Place second building at the same spot — should fail (footprint overlap).
    let second = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(second.is_none(), "overlapping building should be rejected");
}

#[test]
fn test_place_building_blocked_by_partial_overlap() {
    let mut world = setup_world_with_economy();
    assert_world_grid_covers(&world, 110.0, 110.0, 32.0);

    // Place first building at (100, 100) with collision_radius=32.
    let first = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(first.is_some());

    // Place second building slightly offset — still overlapping the footprint.
    let second = place_building(&mut world, None, BUILDING_SOLAR_ID, 110.0, 110.0, 0);
    assert!(
        second.is_none(),
        "partially overlapping building should be rejected"
    );
}

#[test]
fn test_place_building_adjacent_succeeds() {
    let mut world = setup_world_with_economy();
    assert_world_grid_covers(&world, 200.0, 200.0, 32.0);

    // Place first building at (100, 100) with collision_radius=32.
    // Footprint covers roughly (68..132, 68..132).
    let first = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(first.is_some());

    // Place second building far enough away that footprints don't overlap.
    let second = place_building(&mut world, None, BUILDING_SOLAR_ID, 200.0, 200.0, 0);
    assert!(second.is_some(), "non-overlapping building should succeed");
}

#[test]
fn test_place_building_blocked_by_impassable_terrain() {
    let mut world = setup_world_with_economy();
    assert_world_grid_covers(&world, 100.0, 100.0, 32.0);

    // Manually mark a region as impassable (cliff).
    {
        let mut grid = world.resource_mut::<TerrainGrid>();
        for y in 95..=105 {
            for x in 95..=105 {
                grid.set(x, y, SimFloat::ZERO);
            }
        }
    }

    // Try to place building on the cliff — should fail.
    let result = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
    assert!(
        result.is_none(),
        "building on impassable terrain should be rejected"
    );
}

#[test]
fn test_finalize_completed_solar() {
    let mut world = setup_world_with_economy();

    // Manually spawn a "completed" solar: has building UnitType but no BuildSite
    world.spawn((
        Position {
            pos: SimVec3::new(
                SimFloat::from_f32(50.0),
                SimFloat::ZERO,
                SimFloat::from_f32(50.0),
            ),
        },
        Health {
            current: 500,
            max: 500,
        },
        Allegiance { team: 0 },
        UnitType {
            id: BUILDING_SOLAR_ID,
        },
        CollisionRadius {
            radius: SimFloat::from_int(16),
        },
    ));

    finalize_completed_buildings(&mut world);

    // Verify it now has a ResourceProducer
    let count = world
        .query_filtered::<&ResourceProducer, Without<Dead>>()
        .iter(&world)
        .count();
    assert!(
        count >= 1,
        "Expected at least 1 ResourceProducer after finalization"
    );
}
