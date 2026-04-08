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
// Building footprint
// ---------------------------------------------------------------------------

/// Grid cells occupied by a building on the [`TerrainGrid`].
///
/// When the building is placed, these cells are marked impassable.
/// When the building is destroyed (marked [`Dead`]), the cells are restored
/// to their original traversal costs.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct BuildingFootprint {
    /// Grid cells `(x, y)` that this building occupies.
    pub cells: Vec<(usize, usize)>,
    /// Original traversal cost of each cell (same order as `cells`),
    /// so they can be restored when the building is removed.
    pub original_costs: Vec<SimFloat>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "components_tests.rs"]
mod tests;
