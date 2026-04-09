//! Uniform-grid spatial index for broad-phase queries.
//!
//! The grid is rebuilt from scratch each tick. All math is deterministic
//! fixed-point. Cell coordinate computation uses bit shift (cell_size
//! must be a power of 2).

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;

use crate::{SimFloat, SimVec2};

/// 2-D uniform grid that bins entities by their XZ position.
#[derive(Resource, Debug, Clone)]
pub struct SpatialGrid {
    /// Log2 of cell_size. Cell coords = pos.raw() >> (32 + shift).
    cell_shift: u32,
    cell_size_raw: i64,
    width: i32,
    height: i32,
    cells: Vec<Vec<(Entity, u32)>>,
    positions: Vec<SimVec2>,
}

impl SpatialGrid {
    /// Create a new grid. `cell_size` MUST be a power of 2.
    pub fn new(cell_size: SimFloat, width: i32, height: i32) -> Self {
        let total = (width as usize) * (height as usize);
        let cell_size_int = (cell_size.raw() >> 32) as u32;
        debug_assert!(
            cell_size_int.is_power_of_two(),
            "SpatialGrid cell_size must be a power of 2, got {cell_size_int}"
        );
        let cell_shift = cell_size_int.trailing_zeros();

        Self {
            cell_shift,
            cell_size_raw: cell_size.raw(),
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

    /// Cell coordinates via bit shift (no division).
    #[inline]
    fn cell_coords(&self, pos: SimVec2) -> (i32, i32) {
        // pos.raw() is 32.32 fixed-point. Integer part = raw >> 32.
        // Divide by cell_size (power of 2) = shift right by cell_shift more.
        let cx = ((pos.x.raw() >> 32) >> self.cell_shift as i64) as i32;
        let cz = ((pos.y.raw() >> 32) >> self.cell_shift as i64) as i32;
        (cx.clamp(0, self.width - 1), cz.clamp(0, self.height - 1))
    }

    #[inline]
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

    /// Query entities within radius. Uses a callback to avoid Vec allocation.
    /// Uses integer-part-only distance check for the inner loop (avoids i128 multiply).
    #[inline]
    pub fn for_each_in_radius(
        &self,
        center: SimVec2,
        radius: SimFloat,
        mut f: impl FnMut(Entity, SimVec2),
    ) {
        let min = SimVec2::new(center.x - radius, center.y - radius);
        let max = SimVec2::new(center.x + radius, center.y + radius);
        let (min_cx, min_cz) = self.cell_coords(min);
        let (max_cx, max_cz) = self.cell_coords(max);

        // Pre-compute integer-part values for fast i32 distance check.
        // SimFloat is Q32.32: integer part = raw >> 32.
        let cx_i = (center.x.raw() >> 32) as i32;
        let cz_i = (center.y.raw() >> 32) as i32;
        // radius² in integer units. Add 1 for rounding headroom.
        let r_i = (radius.raw() >> 32) as i32;
        let r_sq_i = (r_i as i64 + 1) * (r_i as i64 + 1);

        for cz in min_cz..=max_cz {
            for cx in min_cx..=max_cx {
                let idx = self.cell_index(cx, cz);
                for &(entity, pos_idx) in &self.cells[idx] {
                    let pos = self.positions[pos_idx as usize];

                    // Fast integer-part distance check (no i128 multiply).
                    let px_i = (pos.x.raw() >> 32) as i32;
                    let pz_i = (pos.y.raw() >> 32) as i32;
                    let dx = (px_i - cx_i) as i64;
                    let dz = (pz_i - cz_i) as i64;
                    let dist_sq_i = dx * dx + dz * dz;

                    // Quick reject: if integer distance > radius, skip.
                    // Quick accept: if integer distance < radius - 1, accept.
                    if dist_sq_i > r_sq_i {
                        continue;
                    }

                    // For borderline cases, do the precise SimFloat check.
                    // This path is hit rarely (only for entities near the radius boundary).
                    let radius_sq = radius * radius;
                    if pos.distance_squared(center) <= radius_sq {
                        f(entity, pos);
                    }
                }
            }
        }
    }

    /// Query entities within radius, returning a Vec. Use `for_each_in_radius`
    /// in hot paths to avoid allocation.
    pub fn units_in_radius(&self, center: SimVec2, radius: SimFloat) -> Vec<Entity> {
        let mut result = Vec::new();
        self.for_each_in_radius(center, radius, |e, _| result.push(e));
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

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
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
