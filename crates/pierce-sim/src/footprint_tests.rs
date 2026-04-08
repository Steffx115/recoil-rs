use super::*;
use crate::pathfinding::TerrainGrid;

fn grid(w: usize, h: usize) -> TerrainGrid {
    TerrainGrid::new(w, h, SimFloat::ONE)
}

fn pos(x: i32, y: i32) -> SimVec2 {
    SimVec2::new(SimFloat::from_int(x), SimFloat::from_int(y))
}

#[test]
fn footprint_cells_basic() {
    let g = grid(64, 64);
    let center = pos(10, 10);
    let radius = SimFloat::from_int(2);
    let cells = footprint_cells(&g, center, radius);
    assert!(!cells.is_empty());
    for &(x, y) in &cells {
        assert!(x >= 8 && x <= 12);
        assert!(y >= 8 && y <= 12);
    }
}

#[test]
fn mark_makes_cells_impassable() {
    let mut g = grid(64, 64);
    let center = pos(10, 10);
    let radius = SimFloat::from_int(2);
    let fp = mark_building_footprint(&mut g, center, radius);
    for &(x, y) in &fp.cells {
        assert!(!g.is_passable(x, y));
    }
}

#[test]
fn unmark_restores_costs() {
    let mut g = grid(64, 64);
    let center = pos(10, 10);
    let radius = SimFloat::from_int(2);
    let fp = mark_building_footprint(&mut g, center, radius);
    for &(x, y) in &fp.cells {
        assert!(!g.is_passable(x, y));
    }
    unmark_building_footprint(&mut g, &fp);
    for &(x, y) in &fp.cells {
        assert!(g.is_passable(x, y));
    }
}

#[test]
fn can_place_on_open_ground() {
    let g = grid(64, 64);
    assert!(can_place_building(&g, pos(10, 10), SimFloat::from_int(2)));
}

#[test]
fn cannot_place_on_blocked() {
    let mut g = grid(64, 64);
    mark_building_footprint(&mut g, pos(10, 10), SimFloat::from_int(2));
    assert!(!can_place_building(&g, pos(10, 10), SimFloat::from_int(2)));
}

#[test]
fn footprint_at_grid_edge() {
    let g = grid(20, 20);
    let cells = footprint_cells(&g, pos(0, 0), SimFloat::from_int(2));
    assert!(!cells.is_empty());
    for &(x, y) in &cells {
        assert!(x < 20 && y < 20);
    }
}
