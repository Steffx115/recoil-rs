//! Command queue and unit order processing.
//!
//! Each entity with a [`CommandQueue`] has a FIFO list of [`Command`]s.
//! The [`command_system`] inspects the front command each tick and drives
//! the entity's [`MoveState`] (and, in future sprints, combat / economy
//! states) accordingly.

use std::collections::VecDeque;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::{Component, With, World};
use serde::{Deserialize, Serialize};

use crate::components::{MoveState, Position};
use crate::pathfinding::{find_path, TerrainGrid};
use crate::{SimFloat, SimVec2, SimVec3};

// ---------------------------------------------------------------------------
// Entity serde helper (single Entity as u64)
// ---------------------------------------------------------------------------

mod entity_serde {
    use bevy_ecs::entity::Entity;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(entity: &Entity, serializer: S) -> Result<S::Ok, S::Error> {
        entity.to_bits().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Entity, D::Error> {
        let bits = u64::deserialize(deserializer)?;
        Ok(Entity::from_bits(bits))
    }
}

// ---------------------------------------------------------------------------
// Command enum
// ---------------------------------------------------------------------------

/// A single order that can be issued to a unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Move to a world-space position.
    Move(SimVec3),
    /// Attack a specific target entity.
    Attack(#[serde(with = "entity_serde")] Entity),
    /// Patrol to a point, then return to origin, repeat.
    Patrol(SimVec3),
    /// Follow and protect a friendly entity.
    Guard(#[serde(with = "entity_serde")] Entity),
    /// Halt all actions and clear the queue.
    Stop,
    /// Stop moving but continue engaging targets.
    HoldPosition,
    /// Build a unit of the given type at the given position.
    Build { unit_type: u32, position: SimVec3 },
    /// Reclaim a feature or wreck.
    Reclaim(#[serde(with = "entity_serde")] Entity),
    /// Repair a friendly unit.
    Repair(#[serde(with = "entity_serde")] Entity),
}

// ---------------------------------------------------------------------------
// CommandQueue component
// ---------------------------------------------------------------------------

/// FIFO queue of [`Command`]s attached to an entity.
#[derive(Component, Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandQueue {
    pub commands: VecDeque<Command>,
}

impl CommandQueue {
    /// Append a command to the back of the queue (shift-queue).
    pub fn push(&mut self, cmd: Command) {
        self.commands.push_back(cmd);
    }

    /// Clear the queue and set a single command.
    pub fn replace(&mut self, cmd: Command) {
        self.commands.clear();
        self.commands.push_back(cmd);
    }

    /// Peek at the front command without removing it.
    pub fn current(&self) -> Option<&Command> {
        self.commands.front()
    }

    /// Pop the front command and move to the next one.
    pub fn advance(&mut self) {
        self.commands.pop_front();
    }

    /// Returns `true` when the queue has no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

// ---------------------------------------------------------------------------
// CommandHandler trait and result
// ---------------------------------------------------------------------------

/// Result of executing a command handler for one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResult {
    /// Command is still in progress — keep it at the front of the queue.
    InProgress,
    /// Command completed — advance to the next command in the queue.
    Complete,
    /// Command cleared the entire queue (e.g. Stop).
    QueueCleared,
}

/// Trait for command-specific execution logic.
///
/// Each [`Command`] variant delegates to a handler implementing this trait.
/// The handler reads/writes ECS state via the provided [`World`] and returns
/// a [`CommandResult`] indicating whether the command is still active.
pub trait CommandHandler {
    fn execute(&self, world: &mut World, entity: Entity) -> CommandResult;
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

struct MoveHandler {
    target: SimVec3,
}

impl CommandHandler for MoveHandler {
    fn execute(&self, world: &mut World, entity: Entity) -> CommandResult {
        let state = world.get::<MoveState>(entity).unwrap().clone();
        match state {
            MoveState::Idle => {
                // Compute A* path if terrain grid is available.
                let first_target = compute_pathfinding_waypoints(world, entity, self.target);
                *world.get_mut::<MoveState>(entity).unwrap() = MoveState::MovingTo(first_target);
                CommandResult::InProgress
            }
            MoveState::Arriving => CommandResult::Complete,
            MoveState::MovingTo(_) => {
                // Already moving — let the movement system do its job.
                CommandResult::InProgress
            }
        }
    }
}

struct StopHandler;

impl CommandHandler for StopHandler {
    fn execute(&self, world: &mut World, entity: Entity) -> CommandResult {
        *world.get_mut::<MoveState>(entity).unwrap() = MoveState::Idle;
        world
            .get_mut::<CommandQueue>(entity)
            .unwrap()
            .commands
            .clear();
        CommandResult::QueueCleared
    }
}

struct HoldPositionHandler;

impl CommandHandler for HoldPositionHandler {
    fn execute(&self, world: &mut World, entity: Entity) -> CommandResult {
        *world.get_mut::<MoveState>(entity).unwrap() = MoveState::Idle;
        CommandResult::Complete
    }
}

struct PatrolHandler {
    target: SimVec3,
}

impl CommandHandler for PatrolHandler {
    fn execute(&self, world: &mut World, entity: Entity) -> CommandResult {
        let state = world.get::<MoveState>(entity).unwrap().clone();
        match state {
            MoveState::Arriving => {
                // Arrived at patrol point — push patrol back and advance.
                let patrol_cmd = Command::Patrol(self.target);
                let q = world.get_mut::<CommandQueue>(entity).unwrap();
                let q = q.into_inner();
                q.commands.push_back(patrol_cmd);
                q.commands.pop_front();
                // We manually managed the queue, so report InProgress to
                // avoid a second advance.
                CommandResult::InProgress
            }
            MoveState::Idle => {
                *world.get_mut::<MoveState>(entity).unwrap() = MoveState::MovingTo(self.target);
                CommandResult::InProgress
            }
            MoveState::MovingTo(_) => {
                // Still en route.
                CommandResult::InProgress
            }
        }
    }
}

/// Stub handler for commands whose full systems are not yet implemented.
/// Advances immediately so the queue progresses.
struct StubHandler;

impl CommandHandler for StubHandler {
    fn execute(&self, _world: &mut World, _entity: Entity) -> CommandResult {
        CommandResult::Complete
    }
}

// ---------------------------------------------------------------------------
// Command → handler dispatch
// ---------------------------------------------------------------------------

impl Command {
    /// Return a trait-object handler for this command variant.
    ///
    /// The returned handler encapsulates the command's parameters and
    /// execution logic. This keeps the [`Command`] enum stable for
    /// serialization while allowing each variant's behaviour to be
    /// implemented independently.
    fn handler(&self) -> Box<dyn CommandHandler> {
        match self {
            Command::Move(pos) => Box::new(MoveHandler { target: *pos }),
            Command::Stop => Box::new(StopHandler),
            Command::HoldPosition => Box::new(HoldPositionHandler),
            Command::Patrol(pos) => Box::new(PatrolHandler { target: *pos }),
            Command::Attack(_)
            | Command::Guard(_)
            | Command::Build { .. }
            | Command::Reclaim(_)
            | Command::Repair(_) => Box::new(StubHandler),
        }
    }
}

// ---------------------------------------------------------------------------
// Pathfinding integration
// ---------------------------------------------------------------------------

/// Compute an A* path from the entity's current position to `target`.
///
/// If a path with multiple waypoints is found, the current `Move(target)`
/// command is replaced with a sequence of `Move(waypoint)` commands in the
/// entity's [`CommandQueue`].  Returns the position of the **first waypoint**
/// that the entity should start moving toward.
///
/// Falls back to the original `target` when pathfinding is unavailable or
/// finds no detour.
fn compute_pathfinding_waypoints(world: &mut World, entity: Entity, target: SimVec3) -> SimVec3 {
    let unit_pos = match world.get::<Position>(entity) {
        Some(p) => p.pos,
        None => return target,
    };

    let terrain = match world.get_resource::<TerrainGrid>() {
        Some(t) => t,
        None => return target,
    };

    let start = SimVec2::new(unit_pos.x, unit_pos.z);
    let goal = SimVec2::new(target.x, target.z);

    let path = match find_path(terrain, start, goal) {
        Some(p) if p.len() > 2 => p,
        _ => return target, // straight line or no path — use direct move
    };

    // path[0] is near start (skip it), path[1..last-1] are intermediate
    // waypoints, path[last] is near goal.  We replace the current Move with
    // intermediate waypoints followed by the original target so the unit
    // arrives at the precise requested position.
    let q = world.get_mut::<CommandQueue>(entity).unwrap();
    let q = q.into_inner();
    q.commands.pop_front(); // remove the current Move(target)

    // Push intermediate waypoints (skip first = start, skip last = use original target).
    let intermediates = &path[1..path.len() - 1];
    // Insert in reverse so they end up in order at the front.
    q.commands.push_front(Command::Move(target)); // final destination
    for wp in intermediates.iter().rev() {
        let wp3 = SimVec3::new(wp.x, SimFloat::ZERO, wp.y);
        q.commands.push_front(Command::Move(wp3));
    }

    // First waypoint to move toward.
    let first = &path[1];
    SimVec3::new(first.x, SimFloat::ZERO, first.y)
}

// ---------------------------------------------------------------------------
// command_system
// ---------------------------------------------------------------------------

/// Run one tick of the command processing system.
///
/// For every entity that has both a [`CommandQueue`] and a [`MoveState`],
/// the front command is inspected and the entity's movement state is
/// updated accordingly.  Dispatch is trait-based: each [`Command`] variant
/// provides a [`CommandHandler`] implementation via [`Command::handler`].
pub fn command_system(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, (With<CommandQueue>, With<MoveState>)>()
        .iter(world)
        .collect();

    for entity in entities {
        let Some(cmd) = world
            .get::<CommandQueue>(entity)
            .and_then(|q| q.current().cloned())
        else {
            // Empty queue — do nothing.
            continue;
        };

        let handler = cmd.handler();
        let result = handler.execute(world, entity);

        match result {
            CommandResult::Complete => {
                world.get_mut::<CommandQueue>(entity).unwrap().advance();
            }
            CommandResult::InProgress | CommandResult::QueueCleared => {
                // InProgress: keep command at front.
                // QueueCleared: handler already emptied the queue.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
    use crate::pathfinding::{mark_building_footprint, TerrainGrid};

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
}
