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
#[path = "commands_tests.rs"]
mod tests;
