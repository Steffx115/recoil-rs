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
mod tests {
    use super::*;
    use crate::SimFloat;
    use bevy_ecs::world::World;

    #[test]
    fn animation_state_roundtrip_ecs() {
        let mut world = World::new();
        let state = AnimationState {
            piece_transforms: vec![
                PieceAnimTransform {
                    translate: SimVec3::new(SimFloat::ONE, SimFloat::ZERO, SimFloat::ZERO),
                    rotate: SimVec3::ZERO,
                },
                PieceAnimTransform::default(),
            ],
        };
        let entity = world.spawn(state.clone()).id();
        let read_back = world.get::<AnimationState>(entity).unwrap();
        assert_eq!(read_back.piece_transforms.len(), 2);
        assert_eq!(read_back.piece_transforms[0].translate.x, SimFloat::ONE);
    }

    #[test]
    fn animation_state_serde_roundtrip() {
        let state = AnimationState {
            piece_transforms: vec![
                PieceAnimTransform {
                    translate: SimVec3::new(
                        SimFloat::from_int(10),
                        SimFloat::from_int(20),
                        SimFloat::from_int(30),
                    ),
                    rotate: SimVec3::new(SimFloat::HALF, SimFloat::ZERO, SimFloat::ONE),
                },
                PieceAnimTransform::default(),
                PieceAnimTransform {
                    translate: SimVec3::ZERO,
                    rotate: SimVec3::new(SimFloat::ZERO, SimFloat::ZERO, SimFloat::from_int(3)),
                },
            ],
        };

        let bytes = bincode::serialize(&state).expect("serialize");
        let decoded: AnimationState = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(decoded.piece_transforms.len(), 3);
        assert_eq!(
            decoded.piece_transforms[0].translate.x,
            SimFloat::from_int(10)
        );
        assert_eq!(decoded.piece_transforms[2].rotate.z, SimFloat::from_int(3));
    }

    #[test]
    fn default_animation_state_is_empty() {
        let state = AnimationState::default();
        assert!(state.piece_transforms.is_empty());
    }

    #[test]
    fn piece_anim_transform_default_is_zero() {
        let t = PieceAnimTransform::default();
        assert_eq!(t.translate, SimVec3::ZERO);
        assert_eq!(t.rotate, SimVec3::ZERO);
    }
}
