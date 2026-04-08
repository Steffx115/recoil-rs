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

// ------------------------------------------------------------------
// 6. GPU buffer layout: direction bytes have correct size
// ------------------------------------------------------------------
#[test]
fn direction_field_bytes_correct_size() {
    let grid = open_grid(8, 8);
    let field = compute_flow_field(&grid, pos(4, 4));
    let bytes = field.direction_field_as_bytes();
    assert_eq!(bytes.len(), 8 * 8, "direction bytes should be width*height");
}

// ------------------------------------------------------------------
// 7. GPU buffer layout: cost field bytes have correct size
// ------------------------------------------------------------------
#[test]
fn cost_field_bytes_correct_size() {
    let grid = open_grid(8, 8);
    let field = compute_flow_field(&grid, pos(4, 4));
    let bytes = field.cost_field_as_bytes();
    assert_eq!(
        bytes.len(),
        8 * 8 * 4,
        "cost field bytes should be width*height*sizeof(u32)"
    );
}

// ------------------------------------------------------------------
// 8. Direction encoding: goal cell has DIR_GOAL
// ------------------------------------------------------------------
#[test]
fn goal_cell_has_dir_goal() {
    let grid = open_grid(8, 8);
    let field = compute_flow_field(&grid, pos(4, 4));
    let dir = field.sample_dir(pos(4, 4));
    assert_eq!(dir, DIR_GOAL, "goal cell should have DIR_GOAL");
}

// ------------------------------------------------------------------
// 9. Direction encoding: blocked cells have DIR_BLOCKED
// ------------------------------------------------------------------
#[test]
fn unreachable_cell_has_dir_blocked() {
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
    // Cell (0,0) is cut off.
    assert_eq!(
        field.sample_dir(pos(0, 0)),
        DIR_BLOCKED,
        "unreachable cell should have DIR_BLOCKED"
    );
}

// ------------------------------------------------------------------
// 10. Extract path follows field to goal
// ------------------------------------------------------------------
#[test]
fn extract_path_reaches_goal() {
    let grid = open_grid(10, 10);
    let goal = pos(9, 9);
    let field = compute_flow_field(&grid, goal);
    let path = field.extract_path(pos(0, 0), 100);
    assert!(path.len() >= 2, "path should have multiple steps");
    // Last cell should be (9, 9).
    let last = path.last().unwrap();
    let lx = last.x.to_f64() as usize;
    let ly = last.y.to_f64() as usize;
    assert_eq!((lx, ly), (9, 9), "path should end at goal");
}

// ------------------------------------------------------------------
// 11. Extract path around obstacle
// ------------------------------------------------------------------
#[test]
fn extract_path_around_obstacle() {
    let mut grid = open_grid(10, 10);
    for y in 0..8 {
        grid.set(5, y, SimFloat::ZERO);
    }
    let goal = pos(9, 0);
    let field = compute_flow_field(&grid, goal);
    let path = field.extract_path(pos(0, 0), 200);
    assert!(path.len() >= 2, "path should exist");
    // No waypoint should be on the wall.
    for p in &path {
        let gx = p.x.to_f64() as usize;
        let gy = p.y.to_f64() as usize;
        assert!(
            grid.is_passable(gx, gy),
            "path stepped on wall at ({gx},{gy})"
        );
    }
}

// ------------------------------------------------------------------
// 12. Direction roundtrip: offset → dir → offset
// ------------------------------------------------------------------
#[test]
fn direction_roundtrip() {
    for &(dx, dy, _) in &NEIGHBORS {
        let dir = offset_to_dir(dx, dy);
        let (rx, ry) = dir_to_offset(dir);
        assert_eq!((dx, dy), (rx, ry), "roundtrip failed for ({dx},{dy})");
    }
}
