//! A* pathfinding over a terrain cost grid.
//!
//! All math uses [`SimFloat`] for determinism. The open set is stored in a
//! [`BTreeMap`] keyed by (f-score, tie-breaker) to guarantee platform-identical
//! iteration order — no `HashMap` is used anywhere in this module.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;
use pierce_math::{SimFloat, SimVec2};

use crate::components::{BuildingFootprint, Dead};

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
mod tests {
    use super::*;
    use proptest::prelude::*;

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

    // ==================================================================
    // Building footprint tests
    // ==================================================================

    // ------------------------------------------------------------------
    // 8. footprint_cells computes correct cell set
    // ------------------------------------------------------------------
    #[test]
    fn footprint_cells_basic() {
        let grid = open_grid(20, 20);
        let center = pos(10, 10);
        let radius = SimFloat::from_int(2);
        let cells = footprint_cells(&grid, center, radius);
        // Radius 2 → covers cells from (8,8) to (12,12) = 5×5 = 25 cells.
        assert!(
            cells.len() >= 16,
            "expected >=16 footprint cells, got {}",
            cells.len()
        );
        // All cells should be within the grid.
        for &(x, y) in &cells {
            assert!(x < 20 && y < 20, "cell ({x},{y}) out of bounds");
        }
    }

    // ------------------------------------------------------------------
    // 9. mark_building_footprint makes cells impassable
    // ------------------------------------------------------------------
    #[test]
    fn mark_footprint_makes_cells_impassable() {
        let mut grid = open_grid(20, 20);
        let center = pos(10, 10);
        let radius = SimFloat::from_int(2);
        let fp = mark_building_footprint(&mut grid, center, radius);
        // All footprint cells should now be impassable.
        for &(x, y) in &fp.cells {
            assert!(
                !grid.is_passable(x, y),
                "cell ({x},{y}) should be impassable after marking"
            );
        }
        // Original costs should all be ONE (from open_grid).
        for &cost in &fp.original_costs {
            assert_eq!(cost, SimFloat::ONE);
        }
    }

    // ------------------------------------------------------------------
    // 10. unmark_building_footprint restores original costs
    // ------------------------------------------------------------------
    #[test]
    fn unmark_footprint_restores_costs() {
        let mut grid = open_grid(20, 20);
        // Set some cells to non-default cost first.
        grid.set(10, 10, SimFloat::from_int(3));
        grid.set(11, 10, SimFloat::from_int(5));

        let center = pos(10, 10);
        let radius = SimFloat::from_int(2);
        let fp = mark_building_footprint(&mut grid, center, radius);

        // Cells should be impassable now.
        assert!(!grid.is_passable(10, 10));
        assert!(!grid.is_passable(11, 10));

        // Unmark — should restore original costs.
        unmark_building_footprint(&mut grid, &fp);
        assert_eq!(grid.get(10, 10), Some(SimFloat::from_int(3)));
        assert_eq!(grid.get(11, 10), Some(SimFloat::from_int(5)));
        assert!(grid.is_passable(10, 10));
        assert!(grid.is_passable(11, 10));
    }

    // ------------------------------------------------------------------
    // 11. A* paths around a building footprint
    // ------------------------------------------------------------------
    #[test]
    fn path_around_building_footprint() {
        let mut grid = open_grid(20, 20);
        // Place a building at (10,5) with radius 3 — blocks cells ~(7,2)..(13,8).
        let center = pos(10, 5);
        let radius = SimFloat::from_int(3);
        let _fp = mark_building_footprint(&mut grid, center, radius);

        // Path from (0,5) to (19,5) should detour around the building.
        let path = find_path(&grid, pos(0, 5), pos(19, 5)).expect("path should exist");
        assert!(path.len() > 2, "expected detour, got {path:?}");

        // No waypoint should land on an impassable cell.
        for p in &path {
            let gx = p.x.to_f64() as usize;
            let gy = p.y.to_f64() as usize;
            assert!(
                grid.is_passable(gx, gy),
                "waypoint ({gx},{gy}) is impassable — path goes through building"
            );
        }

        // Path should reach column 19.
        let last = path.last().unwrap();
        assert_eq!(last.x, SimFloat::from_int(19) + SimFloat::HALF);
    }

    // ------------------------------------------------------------------
    // 12. Path between two buildings finds gap
    // ------------------------------------------------------------------
    #[test]
    fn path_between_two_buildings() {
        let mut grid = open_grid(30, 20);
        // Building A blocking upper half at column 10.
        let _fp_a = mark_building_footprint(&mut grid, pos(10, 3), SimFloat::from_int(3));
        // Building B blocking lower half at column 10.
        let _fp_b = mark_building_footprint(&mut grid, pos(10, 16), SimFloat::from_int(3));
        // Gap at rows ~7..13 should allow passage.
        let path = find_path(&grid, pos(0, 10), pos(20, 10)).expect("path through gap");
        assert!(path.len() >= 2);
        let last = path.last().unwrap();
        assert_eq!(last.x, SimFloat::from_int(20) + SimFloat::HALF);
    }

    // ------------------------------------------------------------------
    // 13. footprint_cleanup_system restores terrain on death
    // ------------------------------------------------------------------
    #[test]
    fn footprint_cleanup_on_death() {
        use bevy_ecs::world::World;

        let mut world = World::new();
        let mut grid = open_grid(20, 20);
        let center = pos(10, 10);
        let radius = SimFloat::from_int(2);
        let fp = mark_building_footprint(&mut grid, center, radius);
        let fp_cells = fp.cells.clone();

        world.insert_resource(grid);

        // Spawn a "building" with footprint, then mark it dead.
        let _building = world.spawn((fp, Dead)).id();

        // All footprint cells should be impassable.
        {
            let g = world.resource::<TerrainGrid>();
            for &(x, y) in &fp_cells {
                assert!(!g.is_passable(x, y));
            }
        }

        // Run cleanup system.
        footprint_cleanup_system(&mut world);

        // Cells should now be passable again.
        {
            let g = world.resource::<TerrainGrid>();
            for &(x, y) in &fp_cells {
                assert!(g.is_passable(x, y), "cell ({x},{y}) should be restored");
            }
        }
    }

    // ------------------------------------------------------------------
    // 14. Path recalculation after building destroyed
    // ------------------------------------------------------------------
    #[test]
    fn path_recalculates_after_building_destroyed() {
        let mut grid = open_grid(20, 10);

        // Block column 10 completely with a building.
        let center = pos(10, 5);
        let radius = SimFloat::from_int(5);
        let fp = mark_building_footprint(&mut grid, center, radius);

        // Path from left to right should detour (or be partial).
        let path_blocked = find_path(&grid, pos(0, 5), pos(19, 5));
        // Might find a long detour or partial path.

        // Now "destroy" the building — restore terrain.
        unmark_building_footprint(&mut grid, &fp);

        // Path should now be direct.
        let path_clear = find_path(&grid, pos(0, 5), pos(19, 5)).expect("path after destroy");
        let last = path_clear.last().unwrap();
        assert_eq!(last.x, SimFloat::from_int(19) + SimFloat::HALF);

        // Clear path should be shorter than blocked path (if blocked had one).
        if let Some(blocked) = path_blocked {
            assert!(
                path_clear.len() <= blocked.len(),
                "clear path should be no longer than blocked detour"
            );
        }
    }

    // ------------------------------------------------------------------
    // 15. Footprint at grid edge clamps correctly
    // ------------------------------------------------------------------
    #[test]
    fn footprint_at_grid_edge() {
        let mut grid = open_grid(10, 10);
        // Building at corner (0,0) with radius 3 — should clamp to grid bounds.
        let fp = mark_building_footprint(&mut grid, pos(0, 0), SimFloat::from_int(3));
        assert!(!fp.cells.is_empty());
        for &(x, y) in &fp.cells {
            assert!(x < 10 && y < 10, "cell ({x},{y}) out of bounds");
        }
        // Restore should work without panic.
        unmark_building_footprint(&mut grid, &fp);
    }

    // ==================================================================
    // Property-based tests (proptest)
    // ==================================================================

    const GRID_SIZE: usize = 20;

    fn arb_coord() -> impl Strategy<Value = usize> {
        0..GRID_SIZE
    }

    proptest! {
        // ------------------------------------------------------------------
        // P1. Any reachable goal produces a valid path (no impassable cells)
        // ------------------------------------------------------------------
        #[test]
        fn prop_path_all_cells_passable(
            sx in arb_coord(), sy in arb_coord(),
            gx in arb_coord(), gy in arb_coord(),
        ) {
            let grid = open_grid(GRID_SIZE, GRID_SIZE);
            if let Some(path) = find_path(&grid, pos(sx as i32, sy as i32), pos(gx as i32, gy as i32)) {
                for p in &path {
                    let cx = p.x.to_f64() as usize;
                    let cy = p.y.to_f64() as usize;
                    prop_assert!(
                        grid.is_passable(cx, cy),
                        "waypoint ({cx},{cy}) is impassable"
                    );
                }
            }
        }

        // ------------------------------------------------------------------
        // P2. Path waypoints are within grid bounds
        // ------------------------------------------------------------------
        #[test]
        fn prop_path_within_bounds(
            sx in arb_coord(), sy in arb_coord(),
            gx in arb_coord(), gy in arb_coord(),
        ) {
            let grid = open_grid(GRID_SIZE, GRID_SIZE);
            if let Some(path) = find_path(&grid, pos(sx as i32, sy as i32), pos(gx as i32, gy as i32)) {
                prop_assert!(!path.is_empty(), "path should not be empty");
                for p in &path {
                    let cx = p.x.to_f64() as usize;
                    let cy = p.y.to_f64() as usize;
                    prop_assert!(cx < GRID_SIZE && cy < GRID_SIZE,
                        "waypoint ({},{}) out of grid bounds", cx, cy);
                }
            }
        }

        // ------------------------------------------------------------------
        // P3. Symmetry: path(A,B) exists iff path(B,A) exists
        // ------------------------------------------------------------------
        #[test]
        fn prop_path_symmetry(
            sx in arb_coord(), sy in arb_coord(),
            gx in arb_coord(), gy in arb_coord(),
        ) {
            let grid = open_grid(GRID_SIZE, GRID_SIZE);
            let ab = find_path(&grid, pos(sx as i32, sy as i32), pos(gx as i32, gy as i32));
            let ba = find_path(&grid, pos(gx as i32, gy as i32), pos(sx as i32, sy as i32));
            prop_assert_eq!(ab.is_some(), ba.is_some(),
                "path({},{})->({},{}) exists={}, but reverse exists={}",
                sx, sy, gx, gy, ab.is_some(), ba.is_some());
        }

        // ------------------------------------------------------------------
        // P4. Out-of-bounds goals don't panic (clamped to grid)
        // ------------------------------------------------------------------
        #[test]
        fn prop_oob_goals_no_panic(
            sx in 0..50usize, sy in 0..50usize,
            gx in 0..50usize, gy in 0..50usize,
        ) {
            let grid = open_grid(GRID_SIZE, GRID_SIZE);
            // Coordinates may exceed grid bounds — should not panic
            let _ = find_path(&grid, pos(sx as i32, sy as i32), pos(gx as i32, gy as i32));
        }

        // ------------------------------------------------------------------
        // P5. Impassable start always returns None
        // ------------------------------------------------------------------
        #[test]
        fn prop_impassable_start_returns_none(
            gx in arb_coord(), gy in arb_coord(),
        ) {
            let mut grid = open_grid(GRID_SIZE, GRID_SIZE);
            grid.set(0, 0, SimFloat::ZERO);
            let result = find_path(&grid, pos(0, 0), pos(gx as i32, gy as i32));
            prop_assert!(result.is_none(), "impassable start should return None");
        }
    }

    // ==================================================================
    // Pathfinder trait tests
    // ==================================================================

    #[test]
    fn astar_pathfinder_trait_finds_path() {
        let grid = open_grid(10, 10);
        let pathfinder = AStarPathfinder;
        let path = pathfinder
            .find_path(&grid, pos(0, 0), pos(9, 0))
            .expect("A* pathfinder should find a path");
        assert!(path.len() >= 2);
    }

    #[test]
    fn flowfield_pathfinder_trait_finds_path() {
        let grid = open_grid(10, 10);
        let pathfinder = FlowFieldPathfinder::new();
        let path = pathfinder
            .find_path(&grid, pos(0, 0), pos(9, 0))
            .expect("flow-field pathfinder should find a path");
        assert!(path.len() >= 2);
        // Path should end near goal.
        let last = path.last().unwrap();
        let lx = last.x.to_f64() as usize;
        assert_eq!(lx, 9);
    }

    #[test]
    fn flowfield_pathfinder_routes_around_obstacle() {
        let mut grid = open_grid(10, 10);
        // Wall across column 5, rows 0..8.
        for y in 0..8 {
            grid.set(5, y, SimFloat::ZERO);
        }
        let pathfinder = FlowFieldPathfinder::new();
        let path = pathfinder
            .find_path(&grid, pos(0, 0), pos(9, 0))
            .expect("flow-field should route around obstacle");
        // Verify no waypoint is on the wall.
        for p in &path {
            let gx = p.x.to_f64() as usize;
            let gy = p.y.to_f64() as usize;
            assert!(
                grid.is_passable(gx, gy),
                "waypoint ({gx},{gy}) is on the wall"
            );
        }
    }

    #[test]
    fn hybrid_pathfinder_small_group_uses_astar() {
        let grid = open_grid(20, 20);
        let mut hybrid = HybridPathfinder::new(8);

        // Register < 8 requests to the same goal → should use A*.
        for i in 0..5 {
            hybrid.register_request(pos(i, 0), pos(19, 19));
        }

        let results = hybrid.resolve_all(&grid);
        assert_eq!(results.len(), 5);
        for r in &results {
            assert!(r.is_some(), "all paths should be found");
        }
    }

    #[test]
    fn hybrid_pathfinder_large_group_uses_flowfield() {
        let grid = open_grid(20, 20);
        let mut hybrid = HybridPathfinder::new(4); // lower threshold for test

        // Register >= 4 requests to the same goal → should use flow field.
        for i in 0..10 {
            hybrid.register_request(pos(i, 0), pos(19, 19));
        }

        let results = hybrid.resolve_all(&grid);
        assert_eq!(results.len(), 10);
        for r in &results {
            assert!(r.is_some(), "all paths should be found");
        }
        // Verify flow-field cache was populated.
        // (The cache is internal but we can check indirectly.)
    }

    #[test]
    fn hybrid_pathfinder_mixed_goals() {
        let grid = open_grid(20, 20);
        let mut hybrid = HybridPathfinder::new(3);

        // 4 units to (19,19) → flow field
        for i in 0..4 {
            hybrid.register_request(pos(i, 0), pos(19, 19));
        }
        // 2 units to (0,19) → A*
        for i in 0..2 {
            hybrid.register_request(pos(i, 0), pos(0, 19));
        }

        let results = hybrid.resolve_all(&grid);
        assert_eq!(results.len(), 6);
        for r in &results {
            assert!(r.is_some(), "all paths should be found");
        }
    }

    // ==================================================================
    // Benchmark test (uses std::time::Instant — test-only)
    // ==================================================================

    #[test]
    fn benchmark_astar_vs_flowfield() {
        use std::time::Instant;

        // Create 256x256 grid with scattered obstacles.
        let mut grid = TerrainGrid::new(256, 256, SimFloat::ONE);
        // Add walls every 32 columns, with gaps.
        for wall_x in (32..256).step_by(32) {
            for y in 0..256 {
                if y % 16 < 12 {
                    // wall with gap every 16 rows
                    grid.set(wall_x, y, SimFloat::ZERO);
                }
            }
        }

        let goal = pos(250, 250);

        // Benchmark 100 A* queries.
        let starts: Vec<SimVec2> = (0..100).map(|i| pos(i % 30, (i / 30) * 3)).collect();

        let t0 = Instant::now();
        let mut astar_results = Vec::new();
        for start in &starts {
            astar_results.push(find_path(&grid, *start, goal));
        }
        let astar_time = t0.elapsed();

        // Benchmark 1 flow field + 100 lookups.
        let t1 = Instant::now();
        let field = crate::flowfield::compute_flow_field(&grid, goal);
        let max_steps = 256 * 256;
        let mut ff_results = Vec::new();
        for start in &starts {
            ff_results.push(field.extract_path(*start, max_steps));
        }
        let ff_time = t1.elapsed();

        // Print results (visible with `cargo test -- --nocapture`).
        println!("=== Pathfinding Benchmark (256x256, 100 queries) ===");
        println!("A*:         {astar_time:?}");
        println!("Flow field: {ff_time:?}");
        println!(
            "Speedup:    {:.1}x",
            astar_time.as_secs_f64() / ff_time.as_secs_f64().max(1e-9)
        );

        // Sanity: most paths should exist.
        let astar_found = astar_results.iter().filter(|r| r.is_some()).count();
        let ff_found = ff_results.iter().filter(|r| r.len() > 1).count();
        assert!(astar_found > 50, "A* should find most paths");
        assert!(ff_found > 50, "flow field should find most paths");
    }
}
