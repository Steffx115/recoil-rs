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
#[inline]
fn floor_to_i32(value: SimFloat) -> i32 {
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
    /// Cells marked visible last frame, per team. Used for fast reset
    /// instead of scanning the entire grid.
    #[serde(skip)]
    previously_visible: BTreeMap<u8, Vec<u32>>,
}

impl FogOfWar {
    /// Create a new fog-of-war resource with all cells set to `Unexplored`.
    pub fn new(width: u32, height: u32, teams: &[u8]) -> Self {
        let cell_count = (width as usize) * (height as usize);
        let mut grids = BTreeMap::new();
        let mut previously_visible = BTreeMap::new();
        for &team in teams {
            grids.insert(team, vec![CellVisibility::Unexplored; cell_count]);
            previously_visible.insert(team, Vec::new());
        }
        Self {
            width,
            height,
            grids,
            previously_visible,
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

    /// Reset only the cells that were visible last frame back to `Explored`.
    fn reset_visible(&mut self) {
        let w = self.width as usize;
        for (team, indices) in &self.previously_visible {
            if let Some(grid) = self.grids.get_mut(team) {
                for &idx in indices {
                    let i = idx as usize;
                    if i < grid.len() && grid[i] == CellVisibility::Visible {
                        grid[i] = CellVisibility::Explored;
                    }
                }
            }
        }
        // Clear for this frame's collection.
        for indices in self.previously_visible.values_mut() {
            indices.clear();
        }
        let _ = w;
    }

    /// Mark cell at flat index as `Visible` for `team` and track it.
    #[inline]
    fn mark_visible_idx(&mut self, team: u8, idx: u32) {
        if let Some(grid) = self.grids.get_mut(&team) {
            let i = idx as usize;
            if i < grid.len() && grid[i] != CellVisibility::Visible {
                grid[i] = CellVisibility::Visible;
                if let Some(prev) = self.previously_visible.get_mut(&team) {
                    prev.push(idx);
                }
            }
        }
    }
}

/// Run the fog-of-war system on the given `World`.
///
/// 1. Resets previously visible cells to `Explored`.
/// 2. For each entity with `(Position, SightRange, Allegiance)`,
///    marks all cells within sight range as `Visible` for that team.
pub fn fog_system(world: &mut World, cell_size: SimFloat) {
    let mut fog = match world.remove_resource::<FogOfWar>() {
        Some(f) => f,
        None => return,
    };

    // Step 1: reset only previously visible cells.
    fog.reset_visible();

    // Precompute cell_size as i64 fixed-point for integer math in inner loop.
    let cell_raw = cell_size.raw();
    let half_cell_raw = cell_raw >> 1;
    let w = fog.width();
    let h = fog.height();

    // Step 2: reveal cells around each unit using integer math.
    let mut query = world.query::<(&Position, &SightRange, &Allegiance)>();
    for (pos, sight, allegiance) in query.iter(world) {
        let team = allegiance.team;
        let range = sight.range;

        let cell_x = floor_to_i32(pos.pos.x / cell_size);
        let cell_y = floor_to_i32(pos.pos.z / cell_size);

        let range_cells = floor_to_i32(range / cell_size) + 1;
        // Use raw i64 for squared distance comparison.
        let range_sq_raw = range.raw() as i128 * range.raw() as i128;

        let min_x = (cell_x - range_cells).max(0) as u32;
        let max_x = ((cell_x + range_cells) as u32).min(w.saturating_sub(1));
        let min_y = (cell_y - range_cells).max(0) as u32;
        let max_y = ((cell_y + range_cells) as u32).min(h.saturating_sub(1));

        let pos_x_raw = pos.pos.x.raw();
        let pos_z_raw = pos.pos.z.raw();

        for gy in min_y..=max_y {
            let row_offset = gy * w;
            // Cell center Z in fixed-point.
            let center_z = (gy as i64) * cell_raw + half_cell_raw;
            let dz = center_z - pos_z_raw;

            for gx in min_x..=max_x {
                // Cell center X in fixed-point.
                let center_x = (gx as i64) * cell_raw + half_cell_raw;
                let dx = center_x - pos_x_raw;

                // Squared distance in fixed-point (result is 64.64, compare
                // against range_sq which is also 64.64).
                let dist_sq = (dx as i128) * (dx as i128) + (dz as i128) * (dz as i128);

                if dist_sq <= range_sq_raw {
                    fog.mark_visible_idx(team, row_offset + gx);
                }
            }
        }
    }

    world.insert_resource(fog);
}

/// Check whether a world-space position is visible to a given team.
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
