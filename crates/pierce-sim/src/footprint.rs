//! Building footprint management: placement validation, terrain marking,
//! and cleanup on building death.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use pierce_math::{SimFloat, SimVec2};

use crate::components::{BuildingFootprint, Dead};
use crate::pathfinding::TerrainGrid;

/// Compute grid cells covered by a building at `pos` with the given
/// `collision_radius`.  The footprint is a square of side `radius * 2`
/// centred on the building position (clamped to grid bounds).
pub fn footprint_cells(
    grid: &TerrainGrid,
    pos: SimVec2,
    collision_radius: SimFloat,
) -> Vec<(usize, usize)> {
    let cx = pos.x.to_f64();
    let cz = pos.y.to_f64(); // SimVec2.y maps to world Z
    let r = collision_radius.to_f64();

    let min_x = ((cx - r).floor().max(0.0)) as usize;
    let max_x = ((cx + r).ceil() as usize).min(grid.width().saturating_sub(1));
    let min_y = ((cz - r).floor().max(0.0)) as usize;
    let max_y = ((cz + r).ceil() as usize).min(grid.height().saturating_sub(1));

    let mut cells = Vec::new();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            cells.push((x, y));
        }
    }
    cells
}

/// Mark building footprint cells as impassable on the terrain grid.
///
/// Returns a [`BuildingFootprint`] component storing the cells and their
/// original costs so they can be restored later.
pub fn mark_building_footprint(
    grid: &mut TerrainGrid,
    pos: SimVec2,
    collision_radius: SimFloat,
) -> BuildingFootprint {
    let cells = footprint_cells(grid, pos, collision_radius);
    let original_costs: Vec<SimFloat> = cells
        .iter()
        .map(|&(x, y)| grid.get(x, y).unwrap_or(SimFloat::ONE))
        .collect();

    for &(x, y) in &cells {
        grid.set(x, y, SimFloat::ZERO);
    }

    BuildingFootprint {
        cells,
        original_costs,
    }
}

/// Check if a building can be placed without overlapping existing buildings
/// or impassable terrain.
pub fn can_place_building(
    grid: &TerrainGrid,
    pos: SimVec2,
    collision_radius: SimFloat,
) -> bool {
    let cells = footprint_cells(grid, pos, collision_radius);
    cells.iter().all(|&(x, y)| grid.is_passable(x, y))
}

/// Restore terrain grid cells from a building footprint (when building is
/// destroyed or reclaimed).
pub fn unmark_building_footprint(grid: &mut TerrainGrid, footprint: &BuildingFootprint) {
    for (i, &(x, y)) in footprint.cells.iter().enumerate() {
        if x < grid.width() && y < grid.height() {
            let cost = footprint
                .original_costs
                .get(i)
                .copied()
                .unwrap_or(SimFloat::ONE);
            grid.set(x, y, cost);
        }
    }
}

/// System: restore terrain cells for buildings that have been marked [`Dead`].
///
/// Must run **before** [`cleanup_dead`](crate::lifecycle::cleanup_dead) so the
/// footprint data is still available when we need it.
pub fn footprint_cleanup_system(world: &mut World) {
    let dead_footprints: Vec<(Entity, BuildingFootprint)> = world
        .query_filtered::<(Entity, &BuildingFootprint), bevy_ecs::query::With<Dead>>()
        .iter(world)
        .map(|(e, fp)| (e, fp.clone()))
        .collect();

    if dead_footprints.is_empty() {
        return;
    }

    let mut grid = world.resource_mut::<TerrainGrid>();
    for (_entity, footprint) in &dead_footprints {
        unmark_building_footprint(&mut grid, footprint);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
