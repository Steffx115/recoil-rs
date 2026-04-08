use super::*;
use bevy_ecs::world::World;

fn sf(n: i32) -> SimFloat {
    SimFloat::from_int(n)
}

fn sv2(x: i32, y: i32) -> SimVec2 {
    SimVec2::new(sf(x), sf(y))
}

fn make_grid() -> SpatialGrid {
    SpatialGrid::new(sf(10), 10, 10)
}

#[test]
fn insert_and_query_rect() {
    let mut world = World::new();
    let mut grid = make_grid();
    let e1 = world.spawn_empty().id();
    let e2 = world.spawn_empty().id();
    let e3 = world.spawn_empty().id();
    grid.insert(e1, sv2(5, 5));
    grid.insert(e2, sv2(15, 15));
    grid.insert(e3, sv2(50, 50));
    let result = grid.units_in_rect(sv2(0, 0), sv2(20, 20));
    assert_eq!(result.len(), 2);
    assert!(result.contains(&e1));
    assert!(result.contains(&e2));
}

#[test]
fn insert_and_query_radius() {
    let mut world = World::new();
    let mut grid = make_grid();
    let e1 = world.spawn_empty().id();
    let e2 = world.spawn_empty().id();
    let e3 = world.spawn_empty().id();
    grid.insert(e1, sv2(10, 10));
    grid.insert(e2, sv2(12, 10));
    grid.insert(e3, sv2(50, 50));
    let result = grid.units_in_radius(sv2(10, 10), sf(5));
    assert!(result.contains(&e1));
    assert!(result.contains(&e2));
    assert!(!result.contains(&e3));
}

#[test]
fn clear_empties_grid() {
    let mut world = World::new();
    let mut grid = make_grid();
    let e1 = world.spawn_empty().id();
    grid.insert(e1, sv2(5, 5));
    assert_eq!(grid.len(), 1);
    grid.clear();
    assert!(grid.is_empty());
}

#[test]
fn empty_query_returns_empty() {
    let grid = make_grid();
    assert!(grid.units_in_radius(sv2(50, 50), sf(100)).is_empty());
    assert!(grid.units_in_rect(sv2(0, 0), sv2(99, 99)).is_empty());
}

#[test]
fn entities_on_cell_boundary() {
    let mut world = World::new();
    let mut grid = make_grid();
    let e1 = world.spawn_empty().id();
    grid.insert(e1, sv2(10, 10));
    let result = grid.units_in_rect(sv2(9, 9), sv2(11, 11));
    assert_eq!(result.len(), 1);
}

#[test]
fn position_clamped_to_grid() {
    let mut world = World::new();
    let mut grid = make_grid();
    let e1 = world.spawn_empty().id();
    grid.insert(e1, sv2(-5, -5));
    assert_eq!(grid.len(), 1);
}

#[test]
fn stress_2000_entities() {
    let mut world = World::new();
    let mut grid = make_grid();
    for i in 0..2000 {
        let e = world.spawn_empty().id();
        let x = i % 100;
        let y = i / 100;
        grid.insert(e, sv2(x, y));
    }
    assert_eq!(grid.len(), 2000);
    let result = grid.units_in_rect(sv2(0, 0), sv2(10, 10));
    assert!(!result.is_empty());
    assert!(result.len() < 2000);
    let all = grid.units_in_rect(sv2(0, 0), sv2(99, 99));
    assert_eq!(all.len(), 2000);
}
