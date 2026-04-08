use super::*;
use bevy_ecs::world::World;

/// Helper: create a world with a FogOfWar resource and spawn a unit.
fn setup_world(width: u32, height: u32, teams: &[u8]) -> World {
    let mut world = World::new();
    let fog = FogOfWar::new(width, height, teams);
    world.insert_resource(fog);
    world
}

fn spawn_unit(world: &mut World, x: i32, z: i32, range: i32, team: u8) {
    world.spawn((
        Position {
            pos: SimVec3::new(SimFloat::from_int(x), SimFloat::ZERO, SimFloat::from_int(z)),
        },
        SightRange {
            range: SimFloat::from_int(range),
        },
        Allegiance { team },
    ));
}

const CELL_SIZE: SimFloat = SimFloat::ONE;

#[test]
fn fresh_fog_all_unexplored() {
    let fog = FogOfWar::new(10, 10, &[0, 1]);
    for y in 0..10 {
        for x in 0..10 {
            assert_eq!(fog.get(0, x, y), CellVisibility::Unexplored);
            assert_eq!(fog.get(1, x, y), CellVisibility::Unexplored);
            assert!(!fog.is_visible(0, x, y));
            assert!(!fog.is_explored(0, x, y));
        }
    }
}

#[test]
fn unit_reveals_nearby_cells() {
    let mut world = setup_world(10, 10, &[0]);
    // Place unit at (5, 5) with sight range 2.
    spawn_unit(&mut world, 5, 5, 2, 0);
    fog_system(&mut world, CELL_SIZE);

    let fog = world.resource::<FogOfWar>();
    // The unit's own cell should be visible.
    assert!(fog.is_visible(0, 5, 5));
    // Adjacent cells within range 2 should be visible.
    assert!(fog.is_visible(0, 4, 5));
    assert!(fog.is_visible(0, 6, 5));
    assert!(fog.is_visible(0, 5, 4));
    assert!(fog.is_visible(0, 5, 6));
    // Far corners should not be visible.
    assert!(!fog.is_visible(0, 0, 0));
    assert!(!fog.is_visible(0, 9, 9));
}

#[test]
fn moving_unit_old_cells_become_explored() {
    let mut world = setup_world(20, 20, &[0]);

    // Step 1: unit at (5, 5), sight range 1.
    let entity = world
        .spawn((
            Position {
                pos: SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5)),
            },
            SightRange {
                range: SimFloat::from_int(1),
            },
            Allegiance { team: 0 },
        ))
        .id();
    fog_system(&mut world, CELL_SIZE);

    let fog = world.resource::<FogOfWar>();
    assert!(fog.is_visible(0, 5, 5));

    // Step 2: move unit to (15, 15).
    world.get_mut::<Position>(entity).unwrap().pos = SimVec3::new(
        SimFloat::from_int(15),
        SimFloat::ZERO,
        SimFloat::from_int(15),
    );
    fog_system(&mut world, CELL_SIZE);

    let fog = world.resource::<FogOfWar>();
    // Old cell should be Explored (not Visible, not Unexplored).
    assert_eq!(fog.get(0, 5, 5), CellVisibility::Explored);
    assert!(fog.is_explored(0, 5, 5));
    assert!(!fog.is_visible(0, 5, 5));
    // New cell should be Visible.
    assert!(fog.is_visible(0, 15, 15));
}

#[test]
fn two_teams_independent_fog() {
    let mut world = setup_world(10, 10, &[0, 1]);
    // Team 0 unit at (2, 2), team 1 unit at (7, 7).
    spawn_unit(&mut world, 2, 2, 1, 0);
    spawn_unit(&mut world, 7, 7, 1, 1);
    fog_system(&mut world, CELL_SIZE);

    let fog = world.resource::<FogOfWar>();
    // Team 0 sees (2,2) but not (7,7).
    assert!(fog.is_visible(0, 2, 2));
    assert!(!fog.is_visible(0, 7, 7));
    // Team 1 sees (7,7) but not (2,2).
    assert!(fog.is_visible(1, 7, 7));
    assert!(!fog.is_visible(1, 2, 2));
}

#[test]
fn unit_out_of_range_does_not_reveal() {
    let mut world = setup_world(20, 20, &[0]);
    // Unit at (2, 2) with sight range 1 -- cell (10, 10) is far away.
    spawn_unit(&mut world, 2, 2, 1, 0);
    fog_system(&mut world, CELL_SIZE);

    let fog = world.resource::<FogOfWar>();
    assert!(!fog.is_visible(0, 10, 10));
    assert_eq!(fog.get(0, 10, 10), CellVisibility::Unexplored);
}

#[test]
fn is_entity_visible_utility() {
    let mut world = setup_world(10, 10, &[0]);
    spawn_unit(&mut world, 5, 5, 2, 0);
    fog_system(&mut world, CELL_SIZE);

    let fog = world.resource::<FogOfWar>();
    let visible_pos =
        SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5));
    assert!(is_entity_visible(fog, 0, visible_pos, CELL_SIZE));

    let hidden_pos = SimVec3::new(SimFloat::from_int(0), SimFloat::ZERO, SimFloat::from_int(0));
    assert!(!is_entity_visible(fog, 0, hidden_pos, CELL_SIZE));
}

#[test]
fn out_of_bounds_returns_unexplored() {
    let fog = FogOfWar::new(5, 5, &[0]);
    assert_eq!(fog.get(0, 10, 10), CellVisibility::Unexplored);
    assert_eq!(fog.get(99, 0, 0), CellVisibility::Unexplored);
}
