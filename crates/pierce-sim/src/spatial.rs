//! Uniform-grid spatial index for broad-phase queries.
//!
//! The grid is rebuilt from scratch each tick so that we never mutate
//! cells while iterating them. All math is deterministic fixed-point.

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;

use crate::{SimFloat, SimVec2};

/// 2-D uniform grid that bins entities by their XZ position.
///
/// Stored as a Bevy [`Resource`] and rebuilt every tick from the
/// authoritative [`Position`](crate::Position) components.
///
/// Positions are stored in a flat `Vec` for O(1) indexed lookup during
/// radius/rect queries, replacing the previous `BTreeMap<u64, SimVec2>`.
/// The index into `positions` is stored alongside the entity in each cell.
#[derive(Resource, Debug, Clone)]
pub struct SpatialGrid {
    cell_size: SimFloat,
    width: i32,
    height: i32,
    cells: Vec<Vec<(Entity, u32)>>,
    /// Flat storage of all inserted positions. Index matches insertion order.
    positions: Vec<SimVec2>,
}

impl SpatialGrid {
    pub fn new(cell_size: SimFloat, width: i32, height: i32) -> Self {
        let total = (width as usize) * (height as usize);
        Self {
            cell_size,
            width,
            height,
            cells: vec![Vec::new(); total],
            positions: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
        }
        self.positions.clear();
    }

    fn cell_coords(&self, pos: SimVec2) -> (i32, i32) {
        let cx = (pos.x / self.cell_size).floor().raw() >> 32;
        let cz = (pos.y / self.cell_size).floor().raw() >> 32;
        let cx = (cx as i32).clamp(0, self.width - 1);
        let cz = (cz as i32).clamp(0, self.height - 1);
        (cx, cz)
    }

    fn cell_index(&self, cx: i32, cz: i32) -> usize {
        (cz as usize) * (self.width as usize) + (cx as usize)
    }

    pub fn insert(&mut self, entity: Entity, pos: SimVec2) {
        let (cx, cz) = self.cell_coords(pos);
        let idx = self.cell_index(cx, cz);
        let pos_idx = self.positions.len() as u32;
        self.positions.push(pos);
        self.cells[idx].push((entity, pos_idx));
    }

    pub fn units_in_radius(&self, center: SimVec2, radius: SimFloat) -> Vec<Entity> {
        let radius_sq = radius * radius;
        let min = SimVec2::new(center.x - radius, center.y - radius);
        let max = SimVec2::new(center.x + radius, center.y + radius);
        let (min_cx, min_cz) = self.cell_coords(min);
        let (max_cx, max_cz) = self.cell_coords(max);

        let mut result = Vec::new();
        for cz in min_cz..=max_cz {
            for cx in min_cx..=max_cx {
                let idx = self.cell_index(cx, cz);
                for &(entity, pos_idx) in &self.cells[idx] {
                    let pos = self.positions[pos_idx as usize];
                    if pos.distance_squared(center) <= radius_sq {
                        result.push(entity);
                    }
                }
            }
        }
        result
    }

    pub fn units_in_rect(&self, min_pos: SimVec2, max_pos: SimVec2) -> Vec<Entity> {
        let (min_cx, min_cz) = self.cell_coords(min_pos);
        let (max_cx, max_cz) = self.cell_coords(max_pos);

        let mut result = Vec::new();
        for cz in min_cz..=max_cz {
            for cx in min_cx..=max_cx {
                let idx = self.cell_index(cx, cz);
                for &(entity, pos_idx) in &self.cells[idx] {
                    let pos = self.positions[pos_idx as usize];
                    if pos.x >= min_pos.x
                        && pos.x <= max_pos.x
                        && pos.y >= min_pos.y
                        && pos.y <= max_pos.y
                    {
                        result.push(entity);
                    }
                }
            }
        }
        result
    }

    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

#[cfg(test)]
#[path = "tests/spatial_tests.rs"]
mod tests;
