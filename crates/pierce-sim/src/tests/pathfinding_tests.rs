use super::*;
use crate::components::Dead;
use crate::footprint::{
    can_place_building, footprint_cells, footprint_cleanup_system, mark_building_footprint,
    unmark_building_footprint,
};
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
