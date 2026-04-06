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
mod tests {
    use super::*;
    use crate::{SimFloat, SimVec3};

    /// Create a fresh world with the lifecycle resource initialised.
    fn new_world() -> World {
        let mut world = World::new();
        init_lifecycle(&mut world);
        world
    }

    /// Helper: build a basic set of spawn parameters.
    fn unit_params(team: u8) -> (Position, UnitType, Allegiance, Health) {
        (
            Position { pos: SimVec3::ZERO },
            UnitType { id: 1 },
            Allegiance { team },
            Health {
                current: SimFloat::from_int(100),
                max: SimFloat::from_int(100),
            },
        )
    }

    #[test]
    fn counter_starts_at_one() {
        let counter = SimIdCounter::default();
        assert_eq!(counter.next_id, 1);
    }

    #[test]
    fn allocate_increments() {
        let mut counter = SimIdCounter::default();
        let a = counter.allocate();
        let b = counter.allocate();
        assert_eq!(a.id, 1);
        assert_eq!(b.id, 2);
        assert_eq!(counter.next_id, 3);
    }

    #[test]
    fn spawn_assigns_sequential_ids() {
        let mut world = new_world();
        let (pos, ut, al, hp) = unit_params(1);
        let e1 = spawn_unit(&mut world, pos.clone(), ut.clone(), al.clone(), hp.clone());
        let (pos, ut, al, hp) = unit_params(2);
        let e2 = spawn_unit(&mut world, pos, ut, al, hp);

        let id1 = world.get::<SimId>(e1).unwrap().id;
        let id2 = world.get::<SimId>(e2).unwrap().id;
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn spawn_attaches_all_components() {
        let mut world = new_world();
        let (pos, ut, al, hp) = unit_params(3);
        let entity = spawn_unit(&mut world, pos, ut, al, hp);

        assert!(world.get::<SimId>(entity).is_some());
        assert!(world.get::<Position>(entity).is_some());
        assert!(world.get::<UnitType>(entity).is_some());
        assert!(world.get::<Allegiance>(entity).is_some());
        assert!(world.get::<Health>(entity).is_some());
    }

    #[test]
    fn cleanup_dead_removes_marked_entities() {
        let mut world = new_world();
        let (pos, ut, al, hp) = unit_params(1);
        let entity = spawn_unit(&mut world, pos, ut, al, hp);

        // Mark dead
        world.entity_mut(entity).insert(Dead);
        assert!(world.get::<Dead>(entity).is_some());

        // Cleanup
        cleanup_dead(&mut world);
        assert!(world.get_entity(entity).is_err());
    }

    #[test]
    fn cleanup_dead_leaves_living() {
        let mut world = new_world();
        let (pos, ut, al, hp) = unit_params(1);
        let alive = spawn_unit(&mut world, pos, ut, al, hp);
        let (pos, ut, al, hp) = unit_params(1);
        let doomed = spawn_unit(&mut world, pos, ut, al, hp);

        world.entity_mut(doomed).insert(Dead);
        cleanup_dead(&mut world);

        assert!(world.get_entity(alive).is_ok());
        assert!(world.get_entity(doomed).is_err());
    }

    #[test]
    fn deterministic_id_assignment_across_worlds() {
        /// Run the same spawn/despawn sequence on a fresh world and return
        /// the resulting SimId values in spawn order.
        fn run_sequence() -> Vec<u64> {
            let mut world = World::new();
            init_lifecycle(&mut world);

            let mut ids = Vec::new();

            // Spawn three units
            for team in 0..3 {
                let (pos, ut, al, hp) = super::tests::unit_params(team);
                let e = spawn_unit(&mut world, pos, ut, al, hp);
                ids.push(world.get::<SimId>(e).unwrap().id);
            }

            // Mark the second one dead and clean up
            let second: Vec<Entity> = world
                .query_filtered::<Entity, bevy_ecs::query::With<SimId>>()
                .iter(&world)
                .collect();
            world.entity_mut(second[1]).insert(Dead);
            cleanup_dead(&mut world);

            // Spawn two more
            for team in 3..5 {
                let (pos, ut, al, hp) = super::tests::unit_params(team);
                let e = spawn_unit(&mut world, pos, ut, al, hp);
                ids.push(world.get::<SimId>(e).unwrap().id);
            }

            ids
        }

        let trace_a = run_sequence();
        let trace_b = run_sequence();
        assert_eq!(trace_a, trace_b, "SimId assignment must be deterministic");
        // IDs should be 1..=5 with no gaps or reuse
        assert_eq!(trace_a, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn init_lifecycle_is_idempotent() {
        let mut world = World::new();
        init_lifecycle(&mut world);
        world.resource_mut::<SimIdCounter>().next_id = 42;
        init_lifecycle(&mut world); // should NOT reset
        assert_eq!(world.resource::<SimIdCounter>().next_id, 42);
    }
}
