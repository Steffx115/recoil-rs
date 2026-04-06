//! ECS component types for core game objects.
//!
//! All simulation-facing values use deterministic fixed-point types
//! ([`SimFloat`], [`SimVec3`]) so that replays and checksums stay
//! identical across platforms.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::{SimFloat, SimVec3};

/// Serde helpers for `Vec<Entity>` — serialises each entity as its raw
/// `u64` bits so the list survives replay / checkpoint round-trips.
mod entity_vec_serde {
    use bevy_ecs::entity::Entity;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(entities: &[Entity], serializer: S) -> Result<S::Ok, S::Error> {
        let raw: Vec<u64> = entities.iter().map(|e| e.to_bits()).collect();
        raw.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Entity>, D::Error> {
        let raw = Vec::<u64>::deserialize(deserializer)?;
        Ok(raw.into_iter().map(Entity::from_bits).collect())
    }
}

/// Serde helpers for `Option<Entity>` — serialises as `Option<u64>`.
mod entity_option_serde {
    use bevy_ecs::entity::Entity;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        entity: &Option<Entity>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        entity.map(|e| e.to_bits()).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Entity>, D::Error> {
        let raw = Option::<u64>::deserialize(deserializer)?;
        Ok(raw.map(Entity::from_bits))
    }
}

// ---------------------------------------------------------------------------
// Deterministic entity identity
// ---------------------------------------------------------------------------

/// Deterministic entity identifier that stays consistent across
/// re-simulations and network peers.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimId {
    pub id: u64,
}

// ---------------------------------------------------------------------------
// Spatial
// ---------------------------------------------------------------------------

/// World-space position.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Position {
    pub pos: SimVec3,
}

/// Linear velocity in world-space units per tick.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Velocity {
    pub vel: SimVec3,
}

/// Facing direction expressed as an angle in radians.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Heading {
    pub angle: SimFloat,
}

// ---------------------------------------------------------------------------
// Collision
// ---------------------------------------------------------------------------

/// Circular collision radius for unit-unit overlap detection.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct CollisionRadius {
    pub radius: SimFloat,
}

// ---------------------------------------------------------------------------
// Targeting
// ---------------------------------------------------------------------------

/// The current target of a unit.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Target {
    #[serde(with = "entity_option_serde")]
    pub entity: Option<Entity>,
}

// ---------------------------------------------------------------------------
// Vision
// ---------------------------------------------------------------------------

/// How far a unit can see (in world units).
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SightRange {
    pub range: SimFloat,
}

// ---------------------------------------------------------------------------
// Combat
// ---------------------------------------------------------------------------

/// Hit-points pool.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Health {
    pub current: SimFloat,
    pub max: SimFloat,
}

/// Marker placed on entities that have been destroyed / killed.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct Dead;

/// Applied by paralyzer weapons — prevents the entity from firing.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Stunned {
    pub remaining_frames: u32,
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Index into the game's unit-type registry.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UnitType {
    pub id: u32,
}

/// Which team / player an entity belongs to.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Allegiance {
    pub team: u8,
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

/// Current movement state of an entity.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum MoveState {
    /// Standing still, no movement order.
    Idle,
    /// Actively moving toward a destination.
    MovingTo(SimVec3),
    /// Just arrived at target (transient, will transition to Idle).
    Arriving,
}

/// Tuning parameters that govern how an entity moves.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct MovementParams {
    /// Maximum linear speed in world-units per tick.
    pub max_speed: SimFloat,
    /// Acceleration in world-units per tick per tick.
    pub acceleration: SimFloat,
    /// Maximum turn rate in radians per tick.
    pub turn_rate: SimFloat,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Tracks construction / build progress.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct BuildProgress {
    pub progress: SimFloat,
    pub total_cost: SimFloat,
}

// ---------------------------------------------------------------------------
// Special states
// ---------------------------------------------------------------------------

/// Entity is cloaked and pays an ongoing resource cost.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct Cloaked {
    pub cloak_cost: SimFloat,
}

/// Entity can carry other entities inside it.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct Transport {
    pub capacity: u32,
    #[serde(with = "entity_vec_serde")]
    pub passengers: Vec<Entity>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;

    /// Helper: spawn an entity with the given bundle, then read back the
    /// component and run an assertion closure on it.
    fn roundtrip<C: Component + std::fmt::Debug + Clone>(component: C) -> C {
        let mut world = World::new();
        let entity = world.spawn(component).id();
        world.get::<C>(entity).unwrap().clone()
    }

    #[test]
    fn sim_id_roundtrip() {
        let c = roundtrip(SimId { id: 42 });
        assert_eq!(c.id, 42);
    }

    #[test]
    fn position_roundtrip() {
        let c = roundtrip(Position { pos: SimVec3::ZERO });
        assert_eq!(c.pos, SimVec3::ZERO);
    }

    #[test]
    fn velocity_roundtrip() {
        let v = SimVec3::new(SimFloat::ONE, SimFloat::TWO, SimFloat::ZERO);
        let c = roundtrip(Velocity { vel: v });
        assert_eq!(c.vel, v);
    }

    #[test]
    fn heading_roundtrip() {
        let c = roundtrip(Heading {
            angle: SimFloat::HALF,
        });
        assert_eq!(c.angle, SimFloat::HALF);
    }

    #[test]
    fn health_roundtrip() {
        let c = roundtrip(Health {
            current: SimFloat::from_int(80),
            max: SimFloat::from_int(100),
        });
        assert_eq!(c.current, SimFloat::from_int(80));
        assert_eq!(c.max, SimFloat::from_int(100));
    }

    #[test]
    fn dead_marker() {
        let mut world = World::new();
        let entity = world.spawn(Dead).id();
        assert!(world.get::<Dead>(entity).is_some());
    }

    #[test]
    fn unit_type_roundtrip() {
        let c = roundtrip(UnitType { id: 7 });
        assert_eq!(c.id, 7);
    }

    #[test]
    fn allegiance_roundtrip() {
        let c = roundtrip(Allegiance { team: 3 });
        assert_eq!(c.team, 3);
    }

    #[test]
    fn build_progress_roundtrip() {
        let c = roundtrip(BuildProgress {
            progress: SimFloat::ZERO,
            total_cost: SimFloat::from_int(500),
        });
        assert_eq!(c.progress, SimFloat::ZERO);
        assert_eq!(c.total_cost, SimFloat::from_int(500));
    }

    #[test]
    fn cloaked_roundtrip() {
        let c = roundtrip(Cloaked {
            cloak_cost: SimFloat::ONE,
        });
        assert_eq!(c.cloak_cost, SimFloat::ONE);
    }

    #[test]
    fn transport_roundtrip() {
        let c = roundtrip(Transport {
            capacity: 8,
            passengers: Vec::new(),
        });
        assert_eq!(c.capacity, 8);
        assert!(c.passengers.is_empty());
    }

    #[test]
    fn spawn_all_components_on_one_entity() {
        let mut world = World::new();
        let passenger = world.spawn_empty().id();

        let entity = world
            .spawn((
                SimId { id: 1 },
                Position { pos: SimVec3::ZERO },
                Velocity { vel: SimVec3::ZERO },
                Heading {
                    angle: SimFloat::ZERO,
                },
                Health {
                    current: SimFloat::from_int(100),
                    max: SimFloat::from_int(100),
                },
                UnitType { id: 5 },
                Allegiance { team: 1 },
                BuildProgress {
                    progress: SimFloat::ZERO,
                    total_cost: SimFloat::from_int(200),
                },
                Cloaked {
                    cloak_cost: SimFloat::HALF,
                },
                Transport {
                    capacity: 4,
                    passengers: vec![passenger],
                },
            ))
            .id();

        assert!(world.get::<SimId>(entity).is_some());
        assert!(world.get::<Position>(entity).is_some());
        assert!(world.get::<Velocity>(entity).is_some());
        assert!(world.get::<Heading>(entity).is_some());
        assert!(world.get::<Health>(entity).is_some());
        assert!(world.get::<UnitType>(entity).is_some());
        assert!(world.get::<Allegiance>(entity).is_some());
        assert!(world.get::<BuildProgress>(entity).is_some());
        assert!(world.get::<Cloaked>(entity).is_some());

        let transport = world.get::<Transport>(entity).unwrap();
        assert_eq!(transport.capacity, 4);
        assert_eq!(transport.passengers.len(), 1);
        assert_eq!(transport.passengers[0], passenger);
    }
}
