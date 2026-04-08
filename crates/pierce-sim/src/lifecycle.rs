//! Entity lifecycle management — spawn, despawn, and deterministic ID assignment.
//!
//! Every simulation entity is assigned a [`SimId`] from a monotonically
//! incrementing [`SimIdCounter`] resource. This guarantees deterministic
//! identity across replays and network peers.
//!
//! Entities are never despawned mid-frame. Instead they are marked [`Dead`],
//! and [`cleanup_dead`] removes them at frame end.

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;

use crate::components::{Allegiance, Dead, Health, Position, SimId, UnitType};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Monotonically increasing counter that hands out deterministic [`SimId`]s.
#[derive(Resource, Debug, Clone)]
pub struct SimIdCounter {
    pub next_id: u64,
}

impl Default for SimIdCounter {
    fn default() -> Self {
        Self { next_id: 1 }
    }
}

impl SimIdCounter {
    /// Allocate the next [`SimId`], advancing the counter.
    pub fn allocate(&mut self) -> SimId {
        let id = self.next_id;
        self.next_id += 1;
        SimId { id }
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn a new unit entity with deterministic [`SimId`] assignment.
///
/// The caller provides the initial components; the function pulls the next
/// ID from the world's [`SimIdCounter`] resource.
///
/// # Panics
///
/// Panics if [`SimIdCounter`] has not been inserted into the world.
pub fn spawn_unit(
    world: &mut World,
    position: Position,
    unit_type: UnitType,
    allegiance: Allegiance,
    health: Health,
) -> Entity {
    let sim_id = world.resource_mut::<SimIdCounter>().allocate();
    world
        .spawn((sim_id, position, unit_type, allegiance, health))
        .id()
}

// ---------------------------------------------------------------------------
// Despawn
// ---------------------------------------------------------------------------

/// Remove all entities marked [`Dead`] from the world.
///
/// This should run at the end of each simulation frame so that other
/// systems can still query dying entities during the frame in which they
/// were marked dead.
pub fn cleanup_dead(world: &mut World) {
    let dead_entities: Vec<Entity> = world
        .query_filtered::<Entity, bevy_ecs::query::With<Dead>>()
        .iter(world)
        .collect();

    for entity in dead_entities {
        world.despawn(entity);
    }
}

// ---------------------------------------------------------------------------
// Convenience — init helper
// ---------------------------------------------------------------------------

/// Insert a default [`SimIdCounter`] into the world if one is not already
/// present.
pub fn init_lifecycle(world: &mut World) {
    if !world.contains_resource::<SimIdCounter>() {
        world.insert_resource(SimIdCounter::default());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/lifecycle_tests.rs"]
mod tests;
