//! Fog of war and line-of-sight visibility system.
//!
//! Each team maintains an independent visibility grid. Cells transition
//! through `Unexplored -> Explored -> Visible` as friendly units move
//! around the map. When a unit moves away, cells revert from `Visible`
//! back to `Explored` (never back to `Unexplored`).

use std::collections::BTreeMap;

use bevy_ecs::system::Resource;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::components::{Allegiance, Position, SightRange};
use crate::{SimFloat, SimVec3};

/// Truncate a SimFloat toward negative infinity and return an i32.
///
/// Equivalent to `floor(value)` cast to i32. Uses the raw 32.32
/// fixed-point representation directly (arithmetic right shift).
#[inline]
fn floor_to_i32(value: SimFloat) -> i32 {
    // 32.32 fixed-point: integer part is the upper 32 bits.
    // Arithmetic right shift of a signed i64 rounds toward -inf.
    (value.raw() >> 32) as i32
}

/// Per-cell visibility state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellVisibility {
    /// Never seen by this team.
    Unexplored,
    /// Seen before but not currently in any friendly unit's LOS.
    Explored,
    /// Currently in LOS of at least one friendly unit.
    Visible,
}

/// Per-team fog-of-war grids.
///
/// The grid covers `width * height` cells. Cell `(x, y)` is stored at
/// index `y * width + x`. Each team has its own independent grid.
#[derive(Debug, Clone, Resource, Serialize, Deserialize)]
pub struct FogOfWar {
    width: u32,
    height: u32,
    /// Per-team visibility grids. Uses `BTreeMap` for deterministic
    /// iteration order (no `HashMap` in sim code).
    pub grids: BTreeMap<u8, Vec<CellVisibility>>,
}

impl FogOfWar {
    /// Create a new fog-of-war resource with all cells set to `Unexplored`.
    pub fn new(width: u32, height: u32, teams: &[u8]) -> Self {
        let cell_count = (width as usize) * (height as usize);
        let mut grids = BTreeMap::new();
        for &team in teams {
            grids.insert(team, vec![CellVisibility::Unexplored; cell_count]);
        }
        Self {
            width,
            height,
            grids,
        }
    }

    /// Width of the fog grid in cells.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of the fog grid in cells.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the visibility of cell `(x, y)` for `team`.
    ///
    /// Returns `Unexplored` if the team or coordinates are out of range.
    pub fn get(&self, team: u8, x: u32, y: u32) -> CellVisibility {
        if x >= self.width || y >= self.height {
            return CellVisibility::Unexplored;
        }
        self.grids
            .get(&team)
            .map(|grid| grid[(y as usize) * (self.width as usize) + (x as usize)])
            .unwrap_or(CellVisibility::Unexplored)
    }

    /// Returns `true` if cell `(x, y)` is currently `Visible` for `team`.
    pub fn is_visible(&self, team: u8, x: u32, y: u32) -> bool {
        self.get(team, x, y) == CellVisibility::Visible
    }

    /// Returns `true` if cell `(x, y)` has been seen at least once
    /// (`Explored` or `Visible`) by `team`.
    pub fn is_explored(&self, team: u8, x: u32, y: u32) -> bool {
        matches!(
            self.get(team, x, y),
            CellVisibility::Explored | CellVisibility::Visible
        )
    }

    /// Reset all `Visible` cells to `Explored` for every team.
    /// `Unexplored` cells stay `Unexplored`.
    fn reset_visible(&mut self) {
        for grid in self.grids.values_mut() {
            for cell in grid.iter_mut() {
                if *cell == CellVisibility::Visible {
                    *cell = CellVisibility::Explored;
                }
            }
        }
    }

    /// Mark cell `(x, y)` as `Visible` for `team`.
    fn mark_visible(&mut self, team: u8, x: u32, y: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        if let Some(grid) = self.grids.get_mut(&team) {
            grid[(y as usize) * (self.width as usize) + (x as usize)] = CellVisibility::Visible;
        }
    }
}

/// Run the fog-of-war system on the given `World`.
///
/// 1. Resets all `Visible` cells to `Explored`.
/// 2. For each entity with `(Position, SightRange, Allegiance)`,
///    marks all cells within sight range as `Visible` for that team.
///
/// The `cell_size` parameter controls how world-space positions map to
/// grid cells: `cell_x = floor(pos.x / cell_size)`.
pub fn fog_system(world: &mut World, cell_size: SimFloat) {
    // Scope the resource borrow so we can borrow mutably later.
    let mut fog = match world.remove_resource::<FogOfWar>() {
        Some(f) => f,
        None => return,
    };

    // Step 1: reset Visible -> Explored
    fog.reset_visible();

    // Step 2: reveal cells around each unit
    let mut query = world.query::<(&Position, &SightRange, &Allegiance)>();
    for (pos, sight, allegiance) in query.iter(world) {
        let team = allegiance.team;
        let range = sight.range;

        // Convert world position to grid cell (using x and z as the 2D plane).
        let cell_x = floor_to_i32(pos.pos.x / cell_size);
        let cell_y = floor_to_i32(pos.pos.z / cell_size);

        // Sight range in cells (ceiling to avoid missing edge cells).
        let range_cells = floor_to_i32(range / cell_size) + 1;
        let range_sq = range * range;

        // Iterate bounding box, check circle distance.
        let min_x = (cell_x - range_cells).max(0) as u32;
        let max_x = ((cell_x + range_cells) as u32).min(fog.width().saturating_sub(1));
        let min_y = (cell_y - range_cells).max(0) as u32;
        let max_y = ((cell_y + range_cells) as u32).min(fog.height().saturating_sub(1));

        for gy in min_y..=max_y {
            for gx in min_x..=max_x {
                // Distance from unit's cell to this cell (in world units).
                let dx = SimFloat::from_int(gx as i32) * cell_size + cell_size / SimFloat::TWO
                    - pos.pos.x;
                let dy = SimFloat::from_int(gy as i32) * cell_size + cell_size / SimFloat::TWO
                    - pos.pos.z;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= range_sq {
                    fog.mark_visible(team, gx, gy);
                }
            }
        }
    }

    world.insert_resource(fog);
}

/// Check whether a world-space position is visible to a given team.
///
/// Converts the position to a fog grid cell using `grid_cell_size` and
/// then queries the fog resource.
pub fn is_entity_visible(fog: &FogOfWar, team: u8, pos: SimVec3, grid_cell_size: SimFloat) -> bool {
    let cell_x = floor_to_i32(pos.x / grid_cell_size);
    let cell_y = floor_to_i32(pos.z / grid_cell_size);
    if cell_x < 0 || cell_y < 0 {
        return false;
    }
    fog.is_visible(team, cell_x as u32, cell_y as u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
