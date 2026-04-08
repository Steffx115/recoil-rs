//! Flow-field pathfinding for group movement.
//!
//! A flow field is a grid of direction vectors that units can sample to find
//! their way toward a shared goal. It is computed in two passes:
//!
//! 1. **Integration field** — Dijkstra flood-fill from the goal outward,
//!    recording cost-to-goal for every reachable cell.
//! 2. **Flow field** — Each cell stores a normalised direction vector pointing
//!    toward its lowest-cost neighbour.
//!
//! All arithmetic uses [`SimFloat`] for deterministic, cross-platform results.
//! [`BTreeMap`] is used instead of `HashMap` wherever a map is required.

use std::collections::BTreeMap;

use pierce_math::{SimFloat, SimVec2};

use crate::pathfinding::TerrainGrid;

// ---------------------------------------------------------------------------
// Neighbour offsets (8-connected)
// ---------------------------------------------------------------------------

/// (dx, dy, is_diagonal)
const NEIGHBORS: [(i32, i32, bool); 8] = [
    (-1, -1, true),
    (0, -1, false),
    (1, -1, true),
    (-1, 0, false),
    (1, 0, false),
    (-1, 1, true),
    (0, 1, false),
    (1, 1, true),
];

/// Cost of a diagonal step (sqrt(2) ≈ 1.41421356).
fn diagonal_cost() -> SimFloat {
    SimFloat::from_ratio(141421, 100000)
}

// ---------------------------------------------------------------------------
// IntegrationField
// ---------------------------------------------------------------------------

/// 2-D grid of cost-to-goal values produced by Dijkstra flood-fill.
#[derive(Debug, Clone)]
pub struct IntegrationField {
    width: usize,
    height: usize,
    /// Row-major: `costs[y * width + x]`.
    costs: Vec<SimFloat>,
}

impl IntegrationField {
    /// Build an integration field from the given terrain, flooding outward from
    /// `(goal_x, goal_y)`.
    fn compute(terrain: &TerrainGrid, goal_x: usize, goal_y: usize) -> Self {
        let w = terrain.width();
        let h = terrain.height();
        let mut costs = vec![SimFloat::MAX; w * h];

        // Open set keyed by (cost, tie-breaker) for deterministic ordering.
        let mut open: BTreeMap<(SimFloat, u64), (usize, usize)> = BTreeMap::new();
        let mut tie: u64 = 0;

        // Seed the goal cell.
        if goal_x < w && goal_y < h {
            let idx = goal_y * w + goal_x;
            costs[idx] = SimFloat::ZERO;
            open.insert((SimFloat::ZERO, tie), (goal_x, goal_y));
            tie += 1;
        }

        while let Some((&key, &(cx, cy))) = open.iter().next() {
            open.remove(&key);
            let current_cost = costs[cy * w + cx];

            // If we already found a cheaper route, skip.
            if key.0 > current_cost {
                continue;
            }

            for &(dx, dy, diag) in &NEIGHBORS {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 {
                    continue;
                }
                let nx = nx as usize;
                let ny = ny as usize;
                if nx >= w || ny >= h {
                    continue;
                }

                let cell_cost = match terrain.get(nx, ny) {
                    Some(c) if c > SimFloat::ZERO => c,
                    _ => continue, // impassable
                };

                // Corner-cutting check for diagonal moves.
                if diag {
                    let passable_x = terrain
                        .get(cx.wrapping_add_signed(dx as isize), cy)
                        .is_some_and(|c| c > SimFloat::ZERO);
                    let passable_y = terrain
                        .get(cx, cy.wrapping_add_signed(dy as isize))
                        .is_some_and(|c| c > SimFloat::ZERO);
                    if !passable_x || !passable_y {
                        continue;
                    }
                }

                let step = if diag { diagonal_cost() } else { SimFloat::ONE };
                let tentative = current_cost + step * cell_cost;

                let nidx = ny * w + nx;
                if tentative < costs[nidx] {
                    costs[nidx] = tentative;
                    open.insert((tentative, tie), (nx, ny));
                    tie += 1;
                }
            }
        }

        Self {
            width: w,
            height: h,
            costs,
        }
    }

    /// Get the integration cost for cell `(x, y)`.
    #[inline]
    fn get(&self, x: usize, y: usize) -> SimFloat {
        if x < self.width && y < self.height {
            self.costs[y * self.width + x]
        } else {
            SimFloat::MAX
        }
    }
}

// ---------------------------------------------------------------------------
// FlowField
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Direction encoding (GPU-friendly u8)
// ---------------------------------------------------------------------------

/// Encoded direction as `u8` for GPU-buffer-friendly storage.
///
/// ## WGSL Compute Shader Layout
///
/// The direction field is a contiguous `Vec<u8>` in row-major order:
/// `directions[y * width + x]` gives the direction for cell `(x, y)`.
///
/// Direction encoding (8 cardinal/diagonal + blocked/goal):
///
/// ```text
///   7  0  1
///   6  X  2
///   5  4  3
/// ```
///
/// * `0` = North (0, -1)
/// * `1` = NorthEast (1, -1)
/// * `2` = East (1, 0)
/// * `3` = SouthEast (1, 1)
/// * `4` = South (0, 1)
/// * `5` = SouthWest (-1, 1)
/// * `6` = West (-1, 0)
/// * `7` = NorthWest (-1, -1)
/// * `8` = Goal (no movement needed)
/// * `255` = Blocked/unreachable
pub const DIR_N: u8 = 0;
pub const DIR_NE: u8 = 1;
pub const DIR_E: u8 = 2;
pub const DIR_SE: u8 = 3;
pub const DIR_S: u8 = 4;
pub const DIR_SW: u8 = 5;
pub const DIR_W: u8 = 6;
pub const DIR_NW: u8 = 7;
pub const DIR_GOAL: u8 = 8;
pub const DIR_BLOCKED: u8 = 255;

/// Convert a (dx, dy) offset to a direction `u8`.
fn offset_to_dir(dx: i32, dy: i32) -> u8 {
    match (dx, dy) {
        (0, -1) => DIR_N,
        (1, -1) => DIR_NE,
        (1, 0) => DIR_E,
        (1, 1) => DIR_SE,
        (0, 1) => DIR_S,
        (-1, 1) => DIR_SW,
        (-1, 0) => DIR_W,
        (-1, -1) => DIR_NW,
        _ => DIR_BLOCKED,
    }
}

/// Convert a direction `u8` to a (dx, dy) offset. Returns `(0, 0)` for
/// goal/blocked.
fn dir_to_offset(dir: u8) -> (i32, i32) {
    match dir {
        DIR_N => (0, -1),
        DIR_NE => (1, -1),
        DIR_E => (1, 0),
        DIR_SE => (1, 1),
        DIR_S => (0, 1),
        DIR_SW => (-1, 1),
        DIR_W => (-1, 0),
        DIR_NW => (-1, -1),
        _ => (0, 0),
    }
}

/// Convert a direction `u8` to a normalised `SimVec2`.
fn dir_to_vec(dir: u8) -> SimVec2 {
    let (dx, dy) = dir_to_offset(dir);
    if dx == 0 && dy == 0 {
        return SimVec2::ZERO;
    }
    SimVec2::new(SimFloat::from_int(dx), SimFloat::from_int(dy)).normalize()
}

// ---------------------------------------------------------------------------
// FlowField
// ---------------------------------------------------------------------------

/// 2-D grid of direction vectors stored as `u8` for GPU-buffer compatibility.
///
/// Each cell encodes the direction toward the lowest-cost neighbour using the
/// 8-direction + goal + blocked scheme documented above. A companion
/// `SimVec2`-based direction cache is kept for CPU-side sampling without
/// repeated decode overhead.
///
/// ## GPU Buffer Layout
///
/// * **Cost field** ([`IntegrationField`]): contiguous `Vec<u32>`, row-major,
///   `width * height` elements. Upload via `cost_field_as_bytes()`.
/// * **Direction field**: contiguous `Vec<u8>`, row-major, `width * height`
///   elements. Upload via `direction_field_as_bytes()`.
///
/// Both fields use the same `(y * width + x)` addressing. A WGSL compute
/// shader can read these as `storage` buffers and perform flow-field queries
/// in parallel for thousands of units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowField {
    width: usize,
    height: usize,
    /// Row-major encoded directions (u8). GPU-buffer-friendly.
    direction_bytes: Vec<u8>,
    /// Row-major decoded directions (SimVec2). CPU-side cache.
    directions: Vec<SimVec2>,
    /// Row-major cost-to-goal field stored as u32 (fixed-point raw >> 16).
    /// GPU-friendly contiguous layout.
    cost_field: Vec<u32>,
}

impl FlowField {
    /// Build a flow field from a completed integration field.
    fn from_integration(integration: &IntegrationField) -> Self {
        let w = integration.width;
        let h = integration.height;
        let mut direction_bytes = vec![DIR_BLOCKED; w * h];
        let mut directions = vec![SimVec2::ZERO; w * h];
        let mut cost_field = Vec::with_capacity(w * h);

        // Build cost field (u32). We map SimFloat raw i64 to u32:
        // SimFloat::MAX → u32::MAX (unreachable), else truncate to u32.
        for y in 0..h {
            for x in 0..w {
                let cost = integration.get(x, y);
                let val = if cost == SimFloat::MAX {
                    u32::MAX
                } else {
                    // SimFloat stores as i64 with 32 fractional bits.
                    // Shift right 16 to fit in u32 while preserving relative order.
                    let raw = cost.raw();
                    if raw < 0 {
                        0u32
                    } else {
                        (raw >> 16).min(u32::MAX as i64 - 1) as u32
                    }
                };
                cost_field.push(val);
            }
        }

        for y in 0..h {
            for x in 0..w {
                let cost = integration.get(x, y);
                if cost == SimFloat::MAX {
                    // Unreachable — keep DIR_BLOCKED.
                    continue;
                }
                if cost == SimFloat::ZERO {
                    // Goal cell.
                    direction_bytes[y * w + x] = DIR_GOAL;
                    continue;
                }

                let mut best_cost = SimFloat::MAX;
                let mut best_dx: i32 = 0;
                let mut best_dy: i32 = 0;

                for &(dx, dy, _diag) in &NEIGHBORS {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 {
                        continue;
                    }
                    let nx = nx as usize;
                    let ny = ny as usize;

                    let ncost = integration.get(nx, ny);
                    if ncost < best_cost {
                        best_cost = ncost;
                        best_dx = dx;
                        best_dy = dy;
                    }
                }

                let dir = offset_to_dir(best_dx, best_dy);
                direction_bytes[y * w + x] = dir;
                directions[y * w + x] = dir_to_vec(dir);
            }
        }

        Self {
            width: w,
            height: h,
            direction_bytes,
            directions,
            cost_field,
        }
    }

    /// Sample the flow field at a world-space position.
    ///
    /// The position is truncated to grid-cell coordinates and the direction
    /// stored in that cell is returned. Out-of-bounds or unreachable positions
    /// return `SimVec2::ZERO`.
    pub fn sample(&self, pos: SimVec2) -> SimVec2 {
        let x = pos.x.to_f64() as i64;
        let y = pos.y.to_f64() as i64;
        if x < 0 || y < 0 {
            return SimVec2::ZERO;
        }
        let x = x as usize;
        let y = y as usize;
        if x >= self.width || y >= self.height {
            return SimVec2::ZERO;
        }
        self.directions[y * self.width + x]
    }

    /// Sample the raw direction byte at a world-space position.
    ///
    /// Returns `DIR_BLOCKED` for out-of-bounds or unreachable cells.
    pub fn sample_dir(&self, pos: SimVec2) -> u8 {
        let x = pos.x.to_f64() as i64;
        let y = pos.y.to_f64() as i64;
        if x < 0 || y < 0 {
            return DIR_BLOCKED;
        }
        let x = x as usize;
        let y = y as usize;
        if x >= self.width || y >= self.height {
            return DIR_BLOCKED;
        }
        self.direction_bytes[y * self.width + x]
    }

    /// Grid width.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Grid height.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Return the direction field as a byte slice for GPU buffer upload.
    ///
    /// Layout: row-major `Vec<u8>`, `width * height` elements.
    /// Each byte is a direction code (see module-level direction constants).
    pub fn direction_field_as_bytes(&self) -> &[u8] {
        &self.direction_bytes
    }

    /// Return the cost field as a byte slice for GPU buffer upload.
    ///
    /// Layout: row-major `Vec<u32>`, `width * height` elements, encoded as
    /// little-endian bytes (`4 * width * height` bytes total).
    pub fn cost_field_as_bytes(&self) -> &[u8] {
        // SAFETY: Vec<u32> is contiguous and aligned; we just reinterpret as bytes.
        // This is safe because u32 has no padding and we use the slice's lifetime.
        let ptr = self.cost_field.as_ptr() as *const u8;
        let len = self.cost_field.len() * std::mem::size_of::<u32>();
        // SAFETY: cost_field is a valid contiguous allocation; the returned
        // slice borrows `self` so the data cannot be freed or moved.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// Extract a path by following the flow field from `start` to the goal.
    ///
    /// Returns cell-centre `SimVec2` positions from start to goal (inclusive).
    /// Stops early if the path exceeds `max_steps` to prevent infinite loops
    /// on malformed fields.
    pub fn extract_path(&self, start: SimVec2, max_steps: usize) -> Vec<SimVec2> {
        let mut path = Vec::new();
        let mut cx = start.x.to_f64() as i64;
        let mut cy = start.y.to_f64() as i64;

        for _ in 0..max_steps {
            if cx < 0 || cy < 0 || cx as usize >= self.width || cy as usize >= self.height {
                break;
            }
            let ux = cx as usize;
            let uy = cy as usize;

            let dir = self.direction_bytes[uy * self.width + ux];
            // Add cell centre to path.
            path.push(SimVec2::new(
                SimFloat::from_int(ux as i32) + SimFloat::HALF,
                SimFloat::from_int(uy as i32) + SimFloat::HALF,
            ));

            if dir == DIR_GOAL || dir == DIR_BLOCKED {
                break;
            }

            let (dx, dy) = dir_to_offset(dir);
            cx += dx as i64;
            cy += dy as i64;
        }

        path
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute a flow field for the given terrain with a goal at `goal` (world
/// space). The goal position is truncated to grid-cell indices.
pub fn compute_flow_field(terrain: &TerrainGrid, goal: SimVec2) -> FlowField {
    let gx = (goal.x.to_f64() as usize).min(terrain.width().saturating_sub(1));
    let gy = (goal.y.to_f64() as usize).min(terrain.height().saturating_sub(1));

    let integration = IntegrationField::compute(terrain, gx, gy);
    FlowField::from_integration(&integration)
}

// ---------------------------------------------------------------------------
// FlowFieldCache
// ---------------------------------------------------------------------------

/// Cache of computed flow fields keyed by goal cell coordinates.
///
/// Uses [`BTreeMap`] for deterministic iteration order.
#[derive(Debug, Clone, Default)]
pub struct FlowFieldCache {
    cache: BTreeMap<(usize, usize), FlowField>,
}

impl FlowFieldCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or compute a flow field for the given goal position.
    ///
    /// If a field for the same goal cell already exists it is returned from the
    /// cache; otherwise a new one is computed, stored, and returned.
    pub fn get_or_compute(&mut self, terrain: &TerrainGrid, goal: SimVec2) -> &FlowField {
        let gx = (goal.x.to_f64() as usize).min(terrain.width().saturating_sub(1));
        let gy = (goal.y.to_f64() as usize).min(terrain.height().saturating_sub(1));

        self.cache
            .entry((gx, gy))
            .or_insert_with(|| compute_flow_field(terrain, goal))
    }

    /// Remove all cached fields.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Number of cached fields.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/flowfield_tests.rs"]
mod tests;
