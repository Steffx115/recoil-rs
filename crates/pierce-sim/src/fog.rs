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
///
/// Positions outside the fog grid (negative or beyond grid extents) are
/// treated as **visible** -- the fog grid only restricts visibility for
/// cells it actually covers.
pub fn is_entity_visible(fog: &FogOfWar, team: u8, pos: SimVec3, grid_cell_size: SimFloat) -> bool {
    let cell_x = floor_to_i32(pos.x / grid_cell_size);
    let cell_y = floor_to_i32(pos.z / grid_cell_size);
    if cell_x < 0 || cell_y < 0 {
        return true;
    }
    let cx = cell_x as u32;
    let cy = cell_y as u32;
    if cx >= fog.width() || cy >= fog.height() {
        return true;
    }
    fog.is_visible(team, cx, cy)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/fog_tests.rs"]
mod tests;
