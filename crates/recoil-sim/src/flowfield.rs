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

use recoil_math::{SimFloat, SimVec2};

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

/// 2-D grid of direction vectors. Each cell stores a normalised `SimVec2`
/// pointing toward the lowest-cost neighbour (i.e., toward the goal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowField {
    width: usize,
    height: usize,
    /// Row-major directions.
    directions: Vec<SimVec2>,
}

impl FlowField {
    /// Build a flow field from a completed integration field.
    fn from_integration(integration: &IntegrationField) -> Self {
        let w = integration.width;
        let h = integration.height;
        let mut directions = vec![SimVec2::ZERO; w * h];

        for y in 0..h {
            for x in 0..w {
                // Unreachable cells and the goal cell itself keep ZERO direction.
                let cost = integration.get(x, y);
                if cost == SimFloat::MAX || cost == SimFloat::ZERO {
                    continue;
                }

                let mut best_cost = SimFloat::MAX;
                let mut best_dir = SimVec2::ZERO;

                for &(dx, dy, _diag) in &NEIGHBORS {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 {
                        continue;
                    }
                    let nx = nx as usize;
                    let ny = ny as usize;

                    let cost = integration.get(nx, ny);
                    if cost < best_cost {
                        best_cost = cost;
                        best_dir = SimVec2::new(SimFloat::from_int(dx), SimFloat::from_int(dy));
                    }
                }

                directions[y * w + x] = best_dir.normalize();
            }
        }

        Self {
            width: w,
            height: h,
            directions,
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
    // 1. Simple open field: all cells point toward the goal
    // ------------------------------------------------------------------
    #[test]
    fn open_field_points_toward_goal() {
        let grid = open_grid(8, 8);
        let goal = pos(4, 4);
        let field = compute_flow_field(&grid, goal);

        // Every reachable cell (except the goal itself) should have a non-zero
        // direction, and sampling should yield a vector whose dot product with
        // the vector toward the goal is positive.
        for y in 0..8 {
            for x in 0..8 {
                if x == 4 && y == 4 {
                    continue; // goal cell
                }
                let p = pos(x, y);
                let dir = field.sample(p);
                assert_ne!(dir, SimVec2::ZERO, "cell ({x},{y}) should have a direction");

                // Direction should point *toward* the goal.
                let toward_goal = (goal - p).normalize();
                let dot = dir.dot(toward_goal);
                assert!(
                    dot > SimFloat::ZERO,
                    "cell ({x},{y}): direction should point toward goal (dot={:?})",
                    dot.to_f64()
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // 2. Field with obstacle: flow routes around it
    // ------------------------------------------------------------------
    #[test]
    fn flow_routes_around_obstacle() {
        let mut grid = open_grid(10, 10);
        // Wall across column 5, rows 0..8.
        for y in 0..8 {
            grid.set(5, y, SimFloat::ZERO);
        }

        let goal = pos(8, 0);
        let field = compute_flow_field(&grid, goal);

        // Cell (4, 0) is left of the wall. It cannot go straight right, so its
        // direction must have a positive y component (routing south around the
        // wall) or at least not point directly into the wall.
        let dir = field.sample(pos(4, 0));
        assert_ne!(dir, SimVec2::ZERO, "cell (4,0) should be reachable");

        // Following the flow from (0,0) for many steps should not land us on
        // the wall.
        let mut fx = SimFloat::ZERO;
        let mut fy = SimFloat::ZERO;
        for _ in 0..40 {
            let d = field.sample(SimVec2::new(fx, fy));
            if d == SimVec2::ZERO {
                break;
            }
            fx += d.x;
            fy += d.y;
            // Clamp to grid.
            fx = fx.max(SimFloat::ZERO).min(SimFloat::from_int(9));
            fy = fy.max(SimFloat::ZERO).min(SimFloat::from_int(9));

            let cx = fx.to_f64() as usize;
            let cy = fy.to_f64() as usize;
            assert_ne!(
                grid.get(cx, cy),
                Some(SimFloat::ZERO),
                "flow walked into the wall at ({cx},{cy})"
            );
        }
    }

    // ------------------------------------------------------------------
    // 3. Cache reuse: same goal returns the same field
    // ------------------------------------------------------------------
    #[test]
    fn cache_returns_same_field() {
        let grid = open_grid(6, 6);
        let goal = pos(3, 3);

        let mut cache = FlowFieldCache::new();
        assert!(cache.is_empty());

        let field_a = cache.get_or_compute(&grid, goal).clone();
        assert_eq!(cache.len(), 1);

        let field_b = cache.get_or_compute(&grid, goal).clone();
        assert_eq!(cache.len(), 1); // no new entry

        assert_eq!(field_a, field_b, "cached field must be identical");
    }

    // ------------------------------------------------------------------
    // 4. Determinism: same inputs produce identical output
    // ------------------------------------------------------------------
    #[test]
    fn determinism() {
        let mut grid = open_grid(12, 12);
        // Add some rough terrain.
        for x in 3..7 {
            for y in 0..8 {
                grid.set(x, y, SimFloat::from_int(3));
            }
        }
        grid.set(7, 5, SimFloat::ZERO);
        grid.set(8, 5, SimFloat::ZERO);

        let goal = pos(11, 11);
        let field_a = compute_flow_field(&grid, goal);
        let field_b = compute_flow_field(&grid, goal);
        assert_eq!(field_a, field_b, "flow field must be deterministic");
    }

    // ------------------------------------------------------------------
    // 5. Fully surrounded goal: neighbours still get directions
    // ------------------------------------------------------------------
    #[test]
    fn surrounded_goal_neighbours_unreachable() {
        let mut grid = open_grid(5, 5);
        // Surround (2,2) with impassable cells.
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                grid.set((2 + dx) as usize, (2 + dy) as usize, SimFloat::ZERO);
            }
        }
        let field = compute_flow_field(&grid, pos(2, 2));
        // Cell (0,0) is passable but completely cut off from the goal because
        // the surrounding ring is impassable. Its direction should be ZERO.
        assert_eq!(
            field.sample(pos(0, 0)),
            SimVec2::ZERO,
            "unreachable cell should have zero direction"
        );
    }
}
