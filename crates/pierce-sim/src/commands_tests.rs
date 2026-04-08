use super::*;
use crate::components::*;
use crate::SimFloat;

/// Spawn a minimal entity with CommandQueue and MoveState.
fn spawn_with_queue(world: &mut World, state: MoveState) -> Entity {
    world.spawn((CommandQueue::default(), state)).id()
}

// ---- CommandQueue API tests ----

#[test]
fn push_appends_to_back() {
    let mut q = CommandQueue::default();
    q.push(Command::Move(SimVec3::ZERO));
    q.push(Command::Stop);
    assert_eq!(q.commands.len(), 2);
    assert!(matches!(q.current(), Some(Command::Move(_))));
}

#[test]
fn replace_clears_and_sets() {
    let mut q = CommandQueue::default();
    q.push(Command::Move(SimVec3::ZERO));
    q.push(Command::Stop);
    q.replace(Command::HoldPosition);
    assert_eq!(q.commands.len(), 1);
    assert!(matches!(q.current(), Some(Command::HoldPosition)));
}

#[test]
fn advance_pops_front() {
    let mut q = CommandQueue::default();
    q.push(Command::Move(SimVec3::ZERO));
    q.push(Command::Stop);
    q.advance();
    assert_eq!(q.commands.len(), 1);
    assert!(matches!(q.current(), Some(Command::Stop)));
}

#[test]
fn is_empty_works() {
    let q = CommandQueue::default();
    assert!(q.is_empty());
}

#[test]
fn current_returns_none_when_empty() {
    let q = CommandQueue::default();
    assert!(q.current().is_none());
}

// ---- command_system: Move drives MoveState ----

#[test]
fn move_command_sets_moving_to() {
    let mut world = World::new();
    let target = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);
    let e = spawn_with_queue(&mut world, MoveState::Idle);
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Move(target));

    command_system(&mut world);

    let state = world.get::<MoveState>(e).unwrap();
    assert_eq!(*state, MoveState::MovingTo(target));
}

#[test]
fn move_command_advances_on_arrival() {
    let mut world = World::new();
    let target = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);
    let e = spawn_with_queue(&mut world, MoveState::Arriving);
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Move(target));

    command_system(&mut world);

    let q = world.get::<CommandQueue>(e).unwrap();
    assert!(q.is_empty(), "queue should be empty after arrival advance");
}

// ---- command_system: Stop clears queue and idles ----

#[test]
fn stop_command_clears_queue_and_idles() {
    let mut world = World::new();
    let target = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::ZERO);
    let e = spawn_with_queue(&mut world, MoveState::MovingTo(target));
    {
        let mut q = world.get_mut::<CommandQueue>(e).unwrap();
        q.push(Command::Stop);
        q.push(Command::Move(SimVec3::ZERO)); // should be cleared
    }

    command_system(&mut world);

    let state = world.get::<MoveState>(e).unwrap();
    assert_eq!(*state, MoveState::Idle);
    let q = world.get::<CommandQueue>(e).unwrap();
    assert!(q.is_empty(), "stop should clear entire queue");
}

// ---- command_system: HoldPosition idles without clearing queue ----

#[test]
fn hold_position_idles_without_clearing() {
    let mut world = World::new();
    let e = spawn_with_queue(&mut world, MoveState::MovingTo(SimVec3::ZERO));
    {
        let mut q = world.get_mut::<CommandQueue>(e).unwrap();
        q.push(Command::HoldPosition);
        q.push(Command::Move(SimVec3::ZERO));
    }

    command_system(&mut world);

    let state = world.get::<MoveState>(e).unwrap();
    assert_eq!(*state, MoveState::Idle);
    let q = world.get::<CommandQueue>(e).unwrap();
    // HoldPosition was advanced, Move remains
    assert_eq!(q.commands.len(), 1);
    assert!(matches!(q.current(), Some(Command::Move(_))));
}

// ---- command_system: Patrol loops ----

#[test]
fn patrol_loops_on_arrival() {
    let mut world = World::new();
    let patrol_pos = SimVec3::new(SimFloat::from_int(20), SimFloat::ZERO, SimFloat::ZERO);
    let e = spawn_with_queue(&mut world, MoveState::Idle);
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Patrol(patrol_pos));

    // First tick: should set MovingTo.
    command_system(&mut world);
    let state = world.get::<MoveState>(e).unwrap();
    assert_eq!(*state, MoveState::MovingTo(patrol_pos));

    // Simulate arrival by setting MoveState to Arriving.
    *world.get_mut::<MoveState>(e).unwrap() = MoveState::Arriving;

    // Second tick: patrol should re-queue itself and advance.
    command_system(&mut world);
    let q = world.get::<CommandQueue>(e).unwrap();
    assert_eq!(q.commands.len(), 1, "patrol should re-queue itself");
    assert!(matches!(q.current(), Some(Command::Patrol(_))));
}

// ---- Shift-queue preserves order ----

#[test]
fn shift_queue_preserves_order() {
    let mut world = World::new();
    let a = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::ZERO);
    let b = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);
    let c = SimVec3::new(SimFloat::from_int(15), SimFloat::ZERO, SimFloat::ZERO);

    let e = spawn_with_queue(&mut world, MoveState::Idle);
    {
        let mut q = world.get_mut::<CommandQueue>(e).unwrap();
        q.push(Command::Move(a));
        q.push(Command::Move(b));
        q.push(Command::Move(c));
    }

    // First tick sets MovingTo(a).
    command_system(&mut world);
    assert_eq!(*world.get::<MoveState>(e).unwrap(), MoveState::MovingTo(a));

    // Simulate arrival at a.
    *world.get_mut::<MoveState>(e).unwrap() = MoveState::Arriving;
    command_system(&mut world);

    // Move(a) should be consumed; next is Move(b).
    // Since we just advanced, state was Arriving, so advance happened.
    // Now queue front is Move(b). But state after advance is still
    // Arriving from the world — command_system only processes once.
    // On next tick with Idle (after movement_system clears Arriving):
    *world.get_mut::<MoveState>(e).unwrap() = MoveState::Idle;
    command_system(&mut world);
    assert_eq!(*world.get::<MoveState>(e).unwrap(), MoveState::MovingTo(b));

    // Arrive at b.
    *world.get_mut::<MoveState>(e).unwrap() = MoveState::Arriving;
    command_system(&mut world);
    *world.get_mut::<MoveState>(e).unwrap() = MoveState::Idle;
    command_system(&mut world);
    assert_eq!(*world.get::<MoveState>(e).unwrap(), MoveState::MovingTo(c));

    // Arrive at c.
    *world.get_mut::<MoveState>(e).unwrap() = MoveState::Arriving;
    command_system(&mut world);
    assert!(world.get::<CommandQueue>(e).unwrap().is_empty());
}

// ---- Stub commands advance immediately ----

#[test]
fn stub_commands_advance() {
    let mut world = World::new();
    let target = world.spawn_empty().id();
    let e = spawn_with_queue(&mut world, MoveState::Idle);
    {
        let mut q = world.get_mut::<CommandQueue>(e).unwrap();
        q.push(Command::Attack(target));
        q.push(Command::Guard(target));
        q.push(Command::Build {
            unit_type: 1,
            position: SimVec3::ZERO,
        });
        q.push(Command::Reclaim(target));
        q.push(Command::Repair(target));
    }

    // Each tick should advance one stub command.
    for remaining in (0..5).rev() {
        command_system(&mut world);
        assert_eq!(
            world.get::<CommandQueue>(e).unwrap().commands.len(),
            remaining
        );
    }
}

// ---- Empty queue does nothing ----

#[test]
fn empty_queue_does_nothing() {
    let mut world = World::new();
    let e = spawn_with_queue(&mut world, MoveState::Idle);

    command_system(&mut world);

    assert_eq!(*world.get::<MoveState>(e).unwrap(), MoveState::Idle);
}

// ==================================================================
// Pathfinding-integrated move tests
// ==================================================================

use crate::components::Position;
use crate::footprint::mark_building_footprint;
use crate::pathfinding::TerrainGrid;

/// Spawn an entity with position, command queue, and move state.
fn spawn_with_pos_and_queue(world: &mut World, pos: SimVec3, state: MoveState) -> Entity {
    world
        .spawn((Position { pos }, CommandQueue::default(), state))
        .id()
}

// ---- Move command uses pathfinding around buildings ----

#[test]
fn move_command_paths_around_building() {
    let mut world = World::new();

    // Set up terrain grid with a building blocking the direct path.
    let mut grid = TerrainGrid::new(30, 20, SimFloat::ONE);
    let building_pos = SimVec2::new(SimFloat::from_int(15), SimFloat::from_int(5));
    let _fp = mark_building_footprint(&mut grid, building_pos, SimFloat::from_int(3));
    world.insert_resource(grid);

    // Spawn unit at (5,5) wanting to move to (25,5).
    let start = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5));
    let target = SimVec3::new(
        SimFloat::from_int(25),
        SimFloat::ZERO,
        SimFloat::from_int(5),
    );
    let e = spawn_with_pos_and_queue(&mut world, start, MoveState::Idle);
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Move(target));

    // Process command — should compute path and expand into waypoints.
    command_system(&mut world);

    // Should be moving (not idle).
    let state = world.get::<MoveState>(e).unwrap().clone();
    assert!(
        matches!(state, MoveState::MovingTo(_)),
        "should be moving, got {state:?}"
    );

    // Queue should have been expanded with intermediate waypoints.
    let q = world.get::<CommandQueue>(e).unwrap();
    assert!(
        q.commands.len() >= 2,
        "queue should have waypoints, got {} commands",
        q.commands.len()
    );

    // Last command in queue should be Move(original_target).
    let last_cmd = q.commands.back().unwrap();
    assert!(
        matches!(last_cmd, Command::Move(p) if *p == target),
        "last command should be Move to original target"
    );
}

// ---- Move without obstacles goes direct (no waypoint expansion) ----

#[test]
fn move_command_direct_without_obstacles() {
    let mut world = World::new();

    // Open terrain — no obstacles.
    let grid = TerrainGrid::new(30, 20, SimFloat::ONE);
    world.insert_resource(grid);

    let start = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5));
    let target = SimVec3::new(
        SimFloat::from_int(10),
        SimFloat::ZERO,
        SimFloat::from_int(5),
    );
    let e = spawn_with_pos_and_queue(&mut world, start, MoveState::Idle);
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Move(target));

    command_system(&mut world);

    // On an open grid, A* produces a path of ~2 waypoints (start, end)
    // which triggers the straight-line fallback (path.len() <= 2).
    // The unit should be moving to the target directly.
    let state = world.get::<MoveState>(e).unwrap().clone();
    assert!(matches!(state, MoveState::MovingTo(_)), "should be moving");
}

// ---- Move without terrain grid falls back to direct move ----

#[test]
fn move_command_no_terrain_grid() {
    let mut world = World::new();
    // No TerrainGrid resource inserted.

    let start = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5));
    let target = SimVec3::new(
        SimFloat::from_int(25),
        SimFloat::ZERO,
        SimFloat::from_int(5),
    );
    let e = spawn_with_pos_and_queue(&mut world, start, MoveState::Idle);
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Move(target));

    command_system(&mut world);

    // Should fall back to direct MovingTo.
    assert_eq!(
        *world.get::<MoveState>(e).unwrap(),
        MoveState::MovingTo(target)
    );
}

// ---- Shift-queued moves after pathfinding still work ----

#[test]
fn shift_queue_preserved_after_pathfinding() {
    let mut world = World::new();

    let mut grid = TerrainGrid::new(30, 20, SimFloat::ONE);
    let building_pos = SimVec2::new(SimFloat::from_int(15), SimFloat::from_int(5));
    let _fp = mark_building_footprint(&mut grid, building_pos, SimFloat::from_int(3));
    world.insert_resource(grid);

    let start = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5));
    let target_a = SimVec3::new(
        SimFloat::from_int(25),
        SimFloat::ZERO,
        SimFloat::from_int(5),
    );
    let target_b = SimVec3::new(
        SimFloat::from_int(25),
        SimFloat::ZERO,
        SimFloat::from_int(15),
    );

    let e = spawn_with_pos_and_queue(&mut world, start, MoveState::Idle);
    {
        let mut q = world.get_mut::<CommandQueue>(e).unwrap();
        q.push(Command::Move(target_a));
        q.push(Command::Move(target_b)); // shift-queued
    }

    command_system(&mut world);

    // The queue should contain waypoints for target_a PLUS Move(target_b).
    let q = world.get::<CommandQueue>(e).unwrap();
    let cmds: Vec<_> = q.commands.iter().collect();

    // Find target_b in the queue — it should be the very last command.
    let has_target_b = cmds
        .iter()
        .any(|c| matches!(c, Command::Move(p) if *p == target_b));
    assert!(
        has_target_b,
        "shift-queued Move(target_b) should still be in queue"
    );
}

// ==================================================================
// CommandHandler trait dispatch tests
// ==================================================================

#[test]
fn handler_returns_correct_types() {
    // Verify that each command variant produces an appropriate handler.
    let target = world_spawn_target();

    let commands = vec![
        Command::Move(SimVec3::ZERO),
        Command::Stop,
        Command::HoldPosition,
        Command::Patrol(SimVec3::ZERO),
        Command::Attack(target),
        Command::Guard(target),
        Command::Build {
            unit_type: 1,
            position: SimVec3::ZERO,
        },
        Command::Reclaim(target),
        Command::Repair(target),
    ];

    // Just verify handler() doesn't panic for any variant.
    for cmd in &commands {
        let _handler = cmd.handler();
    }
}

/// Helper: create a dummy entity for handler tests.
fn world_spawn_target() -> Entity {
    let mut world = World::new();
    world.spawn_empty().id()
}

#[test]
fn move_handler_returns_in_progress_from_idle() {
    let mut world = World::new();
    let target = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);
    let e = spawn_with_queue(&mut world, MoveState::Idle);

    let handler = MoveHandler { target };
    let result = handler.execute(&mut world, e);
    assert_eq!(result, CommandResult::InProgress);
    assert_eq!(
        *world.get::<MoveState>(e).unwrap(),
        MoveState::MovingTo(target)
    );
}

#[test]
fn move_handler_returns_complete_on_arriving() {
    let mut world = World::new();
    let target = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);
    let e = spawn_with_queue(&mut world, MoveState::Arriving);

    let handler = MoveHandler { target };
    let result = handler.execute(&mut world, e);
    assert_eq!(result, CommandResult::Complete);
}

#[test]
fn stop_handler_clears_queue() {
    let mut world = World::new();
    let e = spawn_with_queue(&mut world, MoveState::MovingTo(SimVec3::ZERO));
    world
        .get_mut::<CommandQueue>(e)
        .unwrap()
        .push(Command::Move(SimVec3::ZERO));

    let handler = StopHandler;
    let result = handler.execute(&mut world, e);
    assert_eq!(result, CommandResult::QueueCleared);
    assert_eq!(*world.get::<MoveState>(e).unwrap(), MoveState::Idle);
    assert!(world.get::<CommandQueue>(e).unwrap().is_empty());
}

#[test]
fn hold_position_handler_returns_complete() {
    let mut world = World::new();
    let e = spawn_with_queue(&mut world, MoveState::MovingTo(SimVec3::ZERO));

    let handler = HoldPositionHandler;
    let result = handler.execute(&mut world, e);
    assert_eq!(result, CommandResult::Complete);
    assert_eq!(*world.get::<MoveState>(e).unwrap(), MoveState::Idle);
}

#[test]
fn stub_handler_returns_complete() {
    let mut world = World::new();
    let e = spawn_with_queue(&mut world, MoveState::Idle);

    let handler = StubHandler;
    let result = handler.execute(&mut world, e);
    assert_eq!(result, CommandResult::Complete);
}
