//! Animation state ECS component for deterministic piece-based animation.
//!
//! This module lives in `pierce-sim` so that animation state is part of the
//! deterministic simulation and survives checksum / replay round-trips.
//! The actual COB VM and bytecode execution live in `pierce-render`.

use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::SimVec3;

/// Per-piece transform produced by the animation system.
///
/// Translation is in S3O linear units, rotation is in radians (heading,
/// pitch, bank — matching COB axis conventions).
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct PieceAnimTransform {
    /// Translation offset relative to the piece's rest position.
    pub translate: SimVec3,
    /// Euler rotation (x = heading, y = pitch, z = bank) in radians.
    pub rotate: SimVec3,
}

/// ECS component that stores the current animation transforms for every
/// piece of a unit's model.
///
/// The renderer reads this each frame to pose the piece tree before
/// building the vertex buffer.
#[derive(Component, Serialize, Deserialize, Debug, Clone, Default)]
pub struct AnimationState {
    /// One entry per piece, in the same order as the model's piece tree.
    pub piece_transforms: Vec<PieceAnimTransform>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
