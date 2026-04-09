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
use crate::compute::{ComputeBackends, FogGridParams, FogUnitInput};
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
#[derive(Debug, Clone, Resource, Serialize, Deserialize)]
pub struct FogOfWar {
    width: u32,
    height: u32,
    pub grids: BTreeMap<u8, Vec<CellVisibility>>,
    #[serde(skip)]
    previously_visible: BTreeMap<u8, Vec<u32>>,
}

impl FogOfWar {
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

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn get(&self, team: u8, x: u32, y: u32) -> CellVisibility {
        if x >= self.width || y >= self.height {
            return CellVisibility::Unexplored;
        }
        self.grids
            .get(&team)
            .map(|grid| grid[(y as usize) * (self.width as usize) + (x as usize)])
            .unwrap_or(CellVisibility::Unexplored)
    }

    pub fn is_visible(&self, team: u8, x: u32, y: u32) -> bool {
        self.get(team, x, y) == CellVisibility::Visible
    }

    pub fn is_explored(&self, team: u8, x: u32, y: u32) -> bool {
        matches!(
            self.get(team, x, y),
            CellVisibility::Explored | CellVisibility::Visible
        )
    }

    fn reset_visible(&mut self) {
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
        for indices in self.previously_visible.values_mut() {
            indices.clear();
        }
    }

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

    /// Convert grids to raw u8 format for compute backends.
    pub fn grids_as_u8(&self) -> BTreeMap<u8, Vec<u8>> {
        self.grids
            .iter()
            .map(|(&team, grid)| {
                (
                    team,
                    grid.iter()
                        .map(|v| match v {
                            CellVisibility::Unexplored => 0,
                            CellVisibility::Explored => 1,
                            CellVisibility::Visible => 2,
                        })
                        .collect(),
                )
            })
            .collect()
    }

    /// Apply raw u8 grids from compute backend.
    fn apply_u8_grids(&mut self, raw: &BTreeMap<u8, Vec<u8>>) {
        self.previously_visible.values_mut().for_each(|v| v.clear());
        for (&team, raw_grid) in raw {
            if let Some(grid) = self.grids.get_mut(&team) {
                for (i, &val) in raw_grid.iter().enumerate() {
                    if i < grid.len() {
                        grid[i] = match val {
                            0 => CellVisibility::Unexplored,
                            1 => CellVisibility::Explored,
                            _ => CellVisibility::Visible,
                        };
                        if val == 2 {
                            if let Some(prev) = self.previously_visible.get_mut(&team) {
                                prev.push(i as u32);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Sorted team IDs.
    pub fn teams(&self) -> Vec<u8> {
        self.grids.keys().copied().collect()
    }
}

/// Run the fog-of-war system on the given `World`.
///
/// If `ComputeBackends` resource is present, dispatches to it.
/// Otherwise runs the inline CPU implementation.
pub fn fog_system(world: &mut World, cell_size: SimFloat) {
    fog_system_with_flag(world, cell_size, false);
}

/// Fog system with pre-cached backend flag (avoids per-tick TypeId lookup).
pub fn fog_system_with_flag(world: &mut World, cell_size: SimFloat, has_backend: bool) {
    if has_backend {
        fog_system_with_backend(world, cell_size);
    } else {
        fog_system_inline(world, cell_size);
    }
}

/// Fog via compute backend (CPU or GPU).
fn fog_system_with_backend(world: &mut World, cell_size: SimFloat) {
    let fog = match world.remove_resource::<FogOfWar>() {
        Some(f) => f,
        None => return,
    };

    // Gather unit inputs.
    let mut units = Vec::new();
    let mut query = world.query::<(&Position, &SightRange, &Allegiance)>();
    for (pos, sight, allegiance) in query.iter(world) {
        units.push(FogUnitInput {
            pos_x_raw: pos.pos.x.raw(),
            pos_z_raw: pos.pos.z.raw(),
            range_raw: sight.range.raw(),
            team: allegiance.team,
        });
    }

    let params = FogGridParams {
        width: fog.width(),
        height: fog.height(),
        cell_size_raw: cell_size.raw(),
        teams: fog.teams(),
    };
    let prev = fog.grids_as_u8();

    // Use resource_scope to avoid remove/re-insert overhead.
    let result = world.resource_scope(|_world, mut backends: bevy_ecs::prelude::Mut<ComputeBackends>| {
        backends.fog.compute_fog(&params, &units, &prev)
    });

    // Apply results.
    let mut fog = fog;
    fog.apply_u8_grids(&result);
    world.insert_resource(fog);
}

/// Inline CPU fog (original optimized implementation).
fn fog_system_inline(world: &mut World, cell_size: SimFloat) {
    let mut fog = match world.remove_resource::<FogOfWar>() {
        Some(f) => f,
        None => return,
    };

    fog.reset_visible();

    let cell_raw = cell_size.raw();
    let half_cell_raw = cell_raw >> 1;
    let w = fog.width();
    let h = fog.height();

    let mut query = world.query::<(&Position, &SightRange, &Allegiance)>();
    for (pos, sight, allegiance) in query.iter(world) {
        let team = allegiance.team;
        let range = sight.range;

        let cell_x = floor_to_i32(pos.pos.x / cell_size);
        let cell_y = floor_to_i32(pos.pos.z / cell_size);

        let range_cells = floor_to_i32(range / cell_size) + 1;
        let range_sq_raw = range.raw() as i128 * range.raw() as i128;

        let min_x = (cell_x - range_cells).max(0) as u32;
        let max_x = ((cell_x + range_cells) as u32).min(w.saturating_sub(1));
        let min_y = (cell_y - range_cells).max(0) as u32;
        let max_y = ((cell_y + range_cells) as u32).min(h.saturating_sub(1));

        let pos_x_raw = pos.pos.x.raw();
        let pos_z_raw = pos.pos.z.raw();

        for gy in min_y..=max_y {
            let row_offset = gy * w;
            let center_z = (gy as i64) * cell_raw + half_cell_raw;
            let dz = center_z - pos_z_raw;

            for gx in min_x..=max_x {
                let center_x = (gx as i64) * cell_raw + half_cell_raw;
                let dx = center_x - pos_x_raw;

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
