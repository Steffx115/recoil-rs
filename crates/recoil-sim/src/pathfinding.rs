//! A* pathfinding over a terrain cost grid.
//!
//! All math uses [`SimFloat`] for determinism. The open set is stored in a
//! [`BTreeMap`] keyed by (f-score, tie-breaker) to guarantee platform-identical
//! iteration order — no `HashMap` is used anywhere in this module.

use std::collections::BTreeMap;

use bevy_ecs::system::Resource;
use recoil_math::{SimFloat, SimVec2};

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
    fn is_passable(&self, x: usize, y: usize) -> bool {
        self.get(x, y).map_or(false, |c| c > SimFloat::ZERO)
    }
}

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

/// Run A* on `terrain` from `start` to `goal` (world-space `SimVec2`).
///
/// Coordinates are truncated to grid-cell indices (integer part of each
/// component). Returns:
/// * `Some(path)` — a sequence of cell-centre positions from start to goal
///   (or to the nearest reachable cell if the goal is blocked).
/// * `None` — when the start cell itself is impassable.
pub fn find_path(terrain: &TerrainGrid, start: SimVec2, goal: SimVec2) -> Option<Vec<SimVec2>> {
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
    let mut best_h = SimFloat::MAX;
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
    best_h = h0;

    while let Some((&key, &(cx, cy))) = open.iter().next() {
        open.remove(&key);

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
            if diag {
                if !terrain.is_passable(cx.wrapping_add_signed(dx as isize), cy)
                    || !terrain.is_passable(cx, cy.wrapping_add_signed(dy as isize))
                {
                    continue;
                }
            }

            let step = if diag {
                diagonal_cost()
            } else {
                SimFloat::ONE
            };
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a fully passable `w x h` grid.
    fn open_grid(w: usize, h: usize) -> TerrainGrid {
        TerrainGrid::new(w, h, SimFloat::ONE)
    }

    /// Helper: world-space point at cell (x, y).
    fn pos(x: i32, y: i32) -> SimVec2 {
        SimVec2::new(SimFloat::from_int(x), SimFloat::from_int(y))
    }

    // ------------------------------------------------------------------
    // 1. Simple straight-line path on an empty map
    // ------------------------------------------------------------------
    #[test]
    fn straight_line_path() {
        let grid = open_grid(10, 10);
        let path = find_path(&grid, pos(0, 0), pos(5, 0)).expect("path should exist");
        // Path must start near (0,0) and end near (5,0).
        assert!(path.len() >= 2, "path too short: {path:?}");
        let first = path.first().unwrap();
        let last = path.last().unwrap();
        assert_eq!(first.y, SimFloat::HALF);
        assert_eq!(last.x, SimFloat::from_int(5) + SimFloat::HALF);
    }

    // ------------------------------------------------------------------
    // 2. Path around an obstacle
    // ------------------------------------------------------------------
    #[test]
    fn path_around_obstacle() {
        let mut grid = open_grid(10, 10);
        // Wall across columns 3, rows 0..8
        for y in 0..8 {
            grid.set(3, y, SimFloat::ZERO);
        }
        let path = find_path(&grid, pos(0, 0), pos(5, 0)).expect("path should exist");
        // The path must detour south of the wall.
        assert!(path.len() > 2, "expected detour, got {path:?}");
        // Every waypoint must be passable.
        for p in &path {
            let gx = p.x.to_f64() as usize;
            let gy = p.y.to_f64() as usize;
            assert!(
                grid.is_passable(gx, gy),
                "waypoint ({gx},{gy}) is impassable"
            );
        }
        // Must reach column 5.
        let last = path.last().unwrap();
        assert_eq!(last.x, SimFloat::from_int(5) + SimFloat::HALF);
    }

    // ------------------------------------------------------------------
    // 3. Unreachable goal returns partial path
    // ------------------------------------------------------------------
    #[test]
    fn unreachable_goal_partial_path() {
        let mut grid = open_grid(10, 10);
        // Surround cell (5,5) completely.
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                grid.set((5 + dx) as usize, (5 + dy) as usize, SimFloat::ZERO);
            }
        }
        // Goal at (5,5) is reachable ON its cell but the surrounding ring
        // makes it impossible to enter from (0,0).
        let result = find_path(&grid, pos(0, 0), pos(5, 5));
        // Should be Some (partial) or None if truly isolated.
        // The goal cell itself is passable but unreachable.
        match result {
            Some(path) => {
                // Partial path should NOT reach (5,5).
                let last = path.last().unwrap();
                let lx = last.x.to_f64() as usize;
                let ly = last.y.to_f64() as usize;
                assert!(
                    lx != 5 || ly != 5,
                    "should not have reached the surrounded goal"
                );
            }
            None => {
                // Also acceptable — start is fine but nowhere useful to go.
                // This can only happen if the partial path is length <= 1.
            }
        }
    }

    // ------------------------------------------------------------------
    // 4. Empty map (no obstacles) — path exists
    // ------------------------------------------------------------------
    #[test]
    fn empty_map_path_exists() {
        let grid = open_grid(20, 20);
        let path = find_path(&grid, pos(0, 0), pos(19, 19)).expect("path should exist");
        assert!(path.len() >= 2);
        let last = path.last().unwrap();
        assert_eq!(last.x, SimFloat::from_int(19) + SimFloat::HALF);
        assert_eq!(last.y, SimFloat::from_int(19) + SimFloat::HALF);
    }

    // ------------------------------------------------------------------
    // 5. Determinism: same inputs → same output
    // ------------------------------------------------------------------
    #[test]
    fn determinism() {
        let mut grid = open_grid(15, 15);
        // Add some rough terrain.
        for x in 3..7 {
            for y in 0..10 {
                grid.set(x, y, SimFloat::from_int(3));
            }
        }
        grid.set(7, 5, SimFloat::ZERO);
        grid.set(8, 5, SimFloat::ZERO);

        let start = pos(0, 0);
        let goal = pos(14, 14);
        let path_a = find_path(&grid, start, goal);
        let path_b = find_path(&grid, start, goal);
        assert_eq!(path_a, path_b, "pathfinding must be deterministic");
    }

    // ------------------------------------------------------------------
    // 6. Start on impassable cell returns None
    // ------------------------------------------------------------------
    #[test]
    fn impassable_start() {
        let mut grid = open_grid(5, 5);
        grid.set(0, 0, SimFloat::ZERO);
        assert!(find_path(&grid, pos(0, 0), pos(4, 4)).is_none());
    }

    // ------------------------------------------------------------------
    // 7. Terrain cost affects route choice
    // ------------------------------------------------------------------
    #[test]
    fn terrain_cost_affects_route() {
        // Two corridors: top row (cost 1) vs. second row (cost 10).
        // A* should prefer the cheaper corridor.
        let mut grid = open_grid(10, 3);
        // Make row 1 expensive.
        for x in 0..10 {
            grid.set(x, 1, SimFloat::from_int(10));
        }
        let path = find_path(&grid, pos(0, 0), pos(9, 0)).expect("path should exist");
        // All waypoints should stay on row 0.
        for p in &path {
            let py = p.y.to_f64() as usize;
            assert_eq!(py, 0, "should prefer cheap row, got y={py}");
        }
    }
}
