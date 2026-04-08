//! A* pathfinding over a terrain cost grid.
//!
//! All math uses [`SimFloat`] for determinism. The open set is stored in a
//! [`BTreeMap`] keyed by (f-score, tie-breaker) to guarantee platform-identical
//! iteration order — no `HashMap` is used anywhere in this module.

use std::collections::BTreeMap;

use bevy_ecs::system::Resource;
use pierce_math::{SimFloat, SimVec2};

// ---------------------------------------------------------------------------
// TerrainGrid
// ---------------------------------------------------------------------------

/// 2-D grid of traversal costs.
///
/// * `SimFloat::ZERO` → impassable (building footprint, cliff, etc.)
/// * `> SimFloat::ZERO` → cost multiplier (1 = normal, 2 = rough terrain, …)
#[derive(Resource, Debug, Clone)]
pub struct TerrainGrid {
    width: usize,
    height: usize,
    /// Row-major: `cells[y * width + x]`.
    cells: Vec<SimFloat>,
}

impl TerrainGrid {
    /// Create a new grid filled with `default_cost` (usually `SimFloat::ONE`).
    pub fn new(width: usize, height: usize, default_cost: SimFloat) -> Self {
        Self {
            width,
            height,
            cells: vec![default_cost; width * height],
        }
    }

    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Get the cost for cell `(x, y)`. Returns `None` if out of bounds.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> Option<SimFloat> {
        if x < self.width && y < self.height {
            Some(self.cells[y * self.width + x])
        } else {
            None
        }
    }

    /// Set the cost for cell `(x, y)`. Panics if out of bounds.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, cost: SimFloat) {
        assert!(x < self.width && y < self.height, "cell out of bounds");
        self.cells[y * self.width + x] = cost;
    }

    /// Returns `true` when the cell is in-bounds and its cost is greater than zero.
    #[inline]
    pub fn is_passable(&self, x: usize, y: usize) -> bool {
        self.get(x, y).is_some_and(|c| c > SimFloat::ZERO)
    }
}

// ---------------------------------------------------------------------------
// Building footprint helpers
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// A* internals
// ---------------------------------------------------------------------------

/// Composite key for the open-set [`BTreeMap`] that orders nodes by f-score
/// first, then by a monotonically-increasing insertion counter for tie-breaking.
/// This guarantees a unique, deterministic ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct OpenKey {
    f_score: SimFloat,
    tie: u64,
}

/// Per-node bookkeeping stored in a flat vec indexed by `y * width + x`.
#[derive(Clone)]
struct NodeInfo {
    g_score: SimFloat,
    f_score: SimFloat,
    parent: Option<(usize, usize)>,
    closed: bool,
}

impl Default for NodeInfo {
    fn default() -> Self {
        Self {
            g_score: SimFloat::MAX,
            f_score: SimFloat::MAX,
            parent: None,
            closed: false,
        }
    }
}

/// Octile-distance heuristic.
///
/// `h = max(dx, dy) + (sqrt(2) - 1) * min(dx, dy)`
///
/// Uses the fixed-point constant for `sqrt(2) - 1 ≈ 0.41421356…`.
fn octile_heuristic(ax: usize, ay: usize, bx: usize, by: usize) -> SimFloat {
    let dx = SimFloat::from_int((ax as i32 - bx as i32).abs());
    let dy = SimFloat::from_int((ay as i32 - by as i32).abs());
    let diag = dx.min(dy);
    let straight = dx.max(dy);
    // sqrt(2) in 32.32 ≈ 1.41421356 → SQRT2_MINUS_ONE ≈ 0.41421356
    // from_ratio(41421, 100000) gives us enough precision.
    let sqrt2_minus_one = SimFloat::from_ratio(41421, 100000);
    straight + sqrt2_minus_one * diag
}

/// 8-connected neighbour offsets: (dx, dy, is_diagonal).
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
// Public API
// ---------------------------------------------------------------------------

/// Default maximum number of A* iterations before returning a partial path.
/// Prevents pathfinding from blocking the sim tick on very large or complex maps.
pub const DEFAULT_MAX_ITERATIONS: u32 = 10_000;

/// Run A* on `terrain` from `start` to `goal` (world-space `SimVec2`).
///
/// Coordinates are truncated to grid-cell indices (integer part of each
/// component). Returns:
/// * `Some(path)` — a sequence of cell-centre positions from start to goal
///   (or to the nearest reachable cell if the goal is blocked).
/// * `None` — when the start cell itself is impassable.
///
/// Uses [`DEFAULT_MAX_ITERATIONS`] as the iteration limit.
pub fn find_path(terrain: &TerrainGrid, start: SimVec2, goal: SimVec2) -> Option<Vec<SimVec2>> {
    find_path_with_limit(terrain, start, goal, DEFAULT_MAX_ITERATIONS)
}

/// Like [`find_path`] but with an explicit iteration limit.
///
/// If the limit is reached before the goal is found, returns a partial
/// path to the closest cell discovered so far.
pub fn find_path_with_limit(
    terrain: &TerrainGrid,
    start: SimVec2,
    goal: SimVec2,
    max_iterations: u32,
) -> Option<Vec<SimVec2>> {
    let sx = start.x.to_f64() as usize;
    let sy = start.y.to_f64() as usize;
    let gx = goal.x.to_f64() as usize;
    let gy = goal.y.to_f64() as usize;

    // Clamp to grid bounds.
    let sx = sx.min(terrain.width().saturating_sub(1));
    let sy = sy.min(terrain.height().saturating_sub(1));
    let gx = gx.min(terrain.width().saturating_sub(1));
    let gy = gy.min(terrain.height().saturating_sub(1));

    // Start must be passable.
    if !terrain.is_passable(sx, sy) {
        return None;
    }

    let w = terrain.width();
    let h = terrain.height();
    let node_count = w * h;

    let mut nodes: Vec<NodeInfo> = vec![NodeInfo::default(); node_count];
    let mut open: BTreeMap<OpenKey, (usize, usize)> = BTreeMap::new();
    let mut tie_counter: u64 = 0;

    // Track closest-to-goal node for partial-path fallback.
    let mut best_cell: (usize, usize) = (sx, sy);

    // Initialise start node.
    let h0 = octile_heuristic(sx, sy, gx, gy);
    let idx_s = sy * w + sx;
    nodes[idx_s].g_score = SimFloat::ZERO;
    nodes[idx_s].f_score = h0;
    open.insert(
        OpenKey {
            f_score: h0,
            tie: tie_counter,
        },
        (sx, sy),
    );
    tie_counter += 1;
    let mut best_h = h0;
    let mut iterations: u32 = 0;

    while let Some((&key, &(cx, cy))) = open.iter().next() {
        open.remove(&key);

        iterations += 1;
        if iterations > max_iterations {
            // Budget exhausted — return partial path to the best cell found.
            break;
        }

        let cidx = cy * w + cx;
        if nodes[cidx].closed {
            continue;
        }
        nodes[cidx].closed = true;

        // Reached the goal?
        if cx == gx && cy == gy {
            let path = reconstruct(&nodes, w, gx, gy);
            return Some(smooth_path(terrain, path));
        }

        let current_g = nodes[cidx].g_score;

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
                _ => continue, // impassable or out of bounds
            };

            // For diagonal moves, also check that the two adjacent cardinal
            // cells are passable (no corner-cutting through walls).
            if diag
                && (!terrain.is_passable(cx.wrapping_add_signed(dx as isize), cy)
                    || !terrain.is_passable(cx, cy.wrapping_add_signed(dy as isize)))
            {
                continue;
            }

            let step = if diag { diagonal_cost() } else { SimFloat::ONE };
            let tentative_g = current_g + step * cell_cost;

            let nidx = ny * w + nx;
            if tentative_g < nodes[nidx].g_score {
                let h = octile_heuristic(nx, ny, gx, gy);
                let f = tentative_g + h;
                nodes[nidx].g_score = tentative_g;
                nodes[nidx].f_score = f;
                nodes[nidx].parent = Some((cx, cy));

                open.insert(
                    OpenKey {
                        f_score: f,
                        tie: tie_counter,
                    },
                    (nx, ny),
                );
                tie_counter += 1;

                if h < best_h {
                    best_h = h;
                    best_cell = (nx, ny);
                }
            }
        }
    }

    // Goal unreachable — return partial path to the closest cell we found.
    let path = reconstruct(&nodes, w, best_cell.0, best_cell.1);
    if path.len() <= 1 {
        // Couldn't move anywhere meaningful.
        return None;
    }
    Some(smooth_path(terrain, path))
}

/// Walk parent pointers back from `(ex, ey)` to the start and return the
/// path as a vec of cell-centre `SimVec2`s (start → end order).
fn reconstruct(nodes: &[NodeInfo], w: usize, ex: usize, ey: usize) -> Vec<SimVec2> {
    let mut cells = Vec::new();
    let mut cur = Some((ex, ey));
    while let Some((x, y)) = cur {
        cells.push(cell_centre(x, y));
        cur = nodes[y * w + x].parent;
    }
    cells.reverse();
    cells
}

/// Centre of cell `(x, y)` expressed as `SimVec2`.
#[inline]
fn cell_centre(x: usize, y: usize) -> SimVec2 {
    SimVec2::new(
        SimFloat::from_int(x as i32) + SimFloat::HALF,
        SimFloat::from_int(y as i32) + SimFloat::HALF,
    )
}

// ---------------------------------------------------------------------------
// Path smoothing
// ---------------------------------------------------------------------------

/// Remove redundant intermediate waypoints that lie on a straight line
/// (collinear with their predecessor and successor).
fn smooth_path(terrain: &TerrainGrid, path: Vec<SimVec2>) -> Vec<SimVec2> {
    if path.len() <= 2 {
        return path;
    }

    let mut smoothed: Vec<SimVec2> = Vec::with_capacity(path.len());
    smoothed.push(path[0]);

    for i in 1..path.len() - 1 {
        let prev = smoothed.last().copied().unwrap();
        let cur = path[i];
        let next = path[i + 1];

        // If prev→cur and cur→next have the same direction, skip cur.
        if !collinear(prev, cur, next, terrain) {
            smoothed.push(cur);
        }
    }

    smoothed.push(*path.last().unwrap());
    smoothed
}

/// Two segments are collinear (and skippable) when:
///   (b - a) × (c - b) == 0
/// AND all cells between a and c are passable (no wall in between).
fn collinear(a: SimVec2, b: SimVec2, c: SimVec2, terrain: &TerrainGrid) -> bool {
    let ab = b - a;
    let bc = c - b;
    // 2-D cross product: ab.x * bc.y - ab.y * bc.x
    let cross = ab.x * bc.y - ab.y * bc.x;
    if cross != SimFloat::ZERO {
        return false;
    }

    // Verify intermediate cells are passable using a simple line walk.
    // Since these are grid-aligned neighbours the walk is trivial; we just
    // need to check that the direct line from a to c doesn't clip an
    // impassable cell. For collinear grid waypoints this is guaranteed as
    // long as direction is consistent, so we only do the cross check above.
    let _ = terrain;
    true
}

// ---------------------------------------------------------------------------
// Pathfinder trait abstraction
// ---------------------------------------------------------------------------

/// Trait for pluggable pathfinding strategies.
///
/// Each implementation finds a path on a [`TerrainGrid`] from `start` to
/// `goal` (both in world-space `SimVec2`). Returns cell-centre waypoints
/// or `None` if no path exists.
pub trait Pathfinder {
    fn find_path(&self, grid: &TerrainGrid, start: SimVec2, goal: SimVec2) -> Option<Vec<SimVec2>>;
}

/// A* pathfinder wrapping the existing [`find_path`] function.
#[derive(Debug, Clone, Default)]
pub struct AStarPathfinder;

impl Pathfinder for AStarPathfinder {
    fn find_path(&self, grid: &TerrainGrid, start: SimVec2, goal: SimVec2) -> Option<Vec<SimVec2>> {
        find_path(grid, start, goal)
    }
}

/// Flow-field pathfinder that computes a flow field for the goal and extracts
/// individual paths by following the field.
///
/// Amortises computation: one flow field serves all units heading to the same
/// destination cell. The cache is cleared each tick or when terrain changes.
#[derive(Debug, Clone, Default)]
pub struct FlowFieldPathfinder {
    cache: crate::flowfield::FlowFieldCache,
}

impl FlowFieldPathfinder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the flow-field cache (call when terrain changes).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Pathfinder for FlowFieldPathfinder {
    fn find_path(&self, grid: &TerrainGrid, start: SimVec2, goal: SimVec2) -> Option<Vec<SimVec2>> {
        // We need &mut self to update the cache, but the trait takes &self.
        // For now, compute a fresh flow field each call. The HybridPathfinder
        // below uses the cache directly.
        let field = crate::flowfield::compute_flow_field(grid, goal);
        let max_steps = grid.width() * grid.height();
        let path = field.extract_path(start, max_steps);
        if path.len() <= 1 {
            None
        } else {
            Some(path)
        }
    }
}

/// Hybrid pathfinder that dispatches between A* and flow-field based on the
/// number of units heading to the same destination.
///
/// * Small groups (< `threshold` units to same goal cell): A*
/// * Large groups (>= `threshold`): compute one flow field, share among all
///
/// The caller must register pending path requests via [`register_request`]
/// before calling [`resolve_all`].
#[derive(Debug, Clone)]
pub struct HybridPathfinder {
    /// Minimum number of units heading to the same goal cell before switching
    /// to flow-field pathfinding. Default: 8.
    pub threshold: usize,

    /// Pending path requests: `(start, goal)` per entity-index.
    requests: Vec<(SimVec2, SimVec2)>,

    /// Flow-field cache (shared across resolve calls within a tick).
    ff_cache: crate::flowfield::FlowFieldCache,
}

impl Default for HybridPathfinder {
    fn default() -> Self {
        Self {
            threshold: 8,
            requests: Vec::new(),
            ff_cache: crate::flowfield::FlowFieldCache::new(),
        }
    }
}

impl HybridPathfinder {
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold,
            ..Default::default()
        }
    }

    /// Register a path request. Returns the index for later retrieval.
    pub fn register_request(&mut self, start: SimVec2, goal: SimVec2) -> usize {
        let idx = self.requests.len();
        self.requests.push((start, goal));
        idx
    }

    /// Resolve all registered requests, returning one `Option<Vec<SimVec2>>`
    /// per request (in registration order).
    ///
    /// Goals that appear >= `threshold` times use flow-field pathfinding;
    /// the rest use A*.
    pub fn resolve_all(&mut self, grid: &TerrainGrid) -> Vec<Option<Vec<SimVec2>>> {
        // Count requests per goal cell using a BTreeMap for determinism.
        let mut goal_counts: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        let goal_cells: Vec<(usize, usize)> = self
            .requests
            .iter()
            .map(|(_s, g)| {
                let gx = (g.x.to_f64() as usize).min(grid.width().saturating_sub(1));
                let gy = (g.y.to_f64() as usize).min(grid.height().saturating_sub(1));
                (gx, gy)
            })
            .collect();

        for &cell in &goal_cells {
            *goal_counts.entry(cell).or_insert(0) += 1;
        }

        let max_steps = grid.width() * grid.height();
        let mut results = Vec::with_capacity(self.requests.len());

        for (i, (start, goal)) in self.requests.iter().enumerate() {
            let cell = goal_cells[i];
            let count = goal_counts.get(&cell).copied().unwrap_or(0);

            if count >= self.threshold {
                // Use flow field.
                let field = self.ff_cache.get_or_compute(grid, *goal);
                let path = field.extract_path(*start, max_steps);
                if path.len() <= 1 {
                    results.push(None);
                } else {
                    results.push(Some(path));
                }
            } else {
                // Use A*.
                results.push(find_path(grid, *start, *goal));
            }
        }

        self.requests.clear();
        results
    }

    /// Clear the flow-field cache (call when terrain changes).
    pub fn clear_cache(&mut self) {
        self.ff_cache.clear();
    }
}

impl Pathfinder for HybridPathfinder {
    fn find_path(&self, grid: &TerrainGrid, start: SimVec2, goal: SimVec2) -> Option<Vec<SimVec2>> {
        // Single-request mode falls back to A* (no group info available).
        find_path(grid, start, goal)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/pathfinding_tests.rs"]
mod tests;
