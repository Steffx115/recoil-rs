//! Ground movement system.
//!
//! Drives entities that have a [`MoveState`] toward their target position,
//! respecting per-entity [`MovementParams`] (max speed, acceleration, turn
//! rate).  All math uses deterministic [`SimFloat`] fixed-point arithmetic.

use bevy_ecs::prelude::*;

use crate::components::{Heading, MoveState, MovementParams, Position, Velocity};
use crate::SimFloat;
use crate::SimVec3;

/// Distance (squared) at which a unit is considered to have arrived.
/// Using squared comparison avoids a sqrt per entity per tick.
const ARRIVAL_THRESHOLD_SQ: SimFloat = SimFloat::ONE; // 1.0 world-unit squared

/// Normalize an angle into the range (-PI, PI].
#[inline]
fn wrap_angle(mut angle: SimFloat) -> SimFloat {
    while angle > SimFloat::PI {
        angle -= SimFloat::TAU;
    }
    while angle <= -SimFloat::PI {
        angle += SimFloat::TAU;
    }
    angle
}

/// Run one tick of the ground movement system.
///
/// For every entity that has the full movement component set, this function:
/// - Zeroes velocity for `Idle` / `Arriving` entities.
/// - Turns toward the target and accelerates for `MovingTo` entities.
/// - Integrates position from velocity.
/// - Transitions `MovingTo` -> `Arriving` -> `Idle` on arrival.
pub fn movement_system(world: &mut World) {
    // Collect entity ids first to satisfy borrow checker (we need mutable
    // access to multiple components on the same entity).
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, (
            With<Position>,
            With<Velocity>,
            With<Heading>,
            With<MoveState>,
            With<MovementParams>,
        )>()
        .iter(world)
        .collect();

    for entity in entities {
        // Read current state ------------------------------------------------
        let pos = world.get::<Position>(entity).unwrap().pos;
        let heading = world.get::<Heading>(entity).unwrap().angle;
        let params = world.get::<MovementParams>(entity).unwrap().clone();
        let state = world.get::<MoveState>(entity).unwrap().clone();

        match state {
            MoveState::Idle => {
                // Stand still.
                world.get_mut::<Velocity>(entity).unwrap().vel = SimVec3::ZERO;
            }
            MoveState::Arriving => {
                // Transient state: stop and go idle.
                world.get_mut::<Velocity>(entity).unwrap().vel = SimVec3::ZERO;
                *world.get_mut::<MoveState>(entity).unwrap() = MoveState::Idle;
            }
            MoveState::MovingTo(target) => {
                let delta = target - pos;
                let dist_sq = delta.length_squared();

                // ---- Arrival check ----
                if dist_sq <= ARRIVAL_THRESHOLD_SQ {
                    world.get_mut::<Velocity>(entity).unwrap().vel = SimVec3::ZERO;
                    world.get_mut::<Position>(entity).unwrap().pos = target;
                    *world.get_mut::<MoveState>(entity).unwrap() = MoveState::Arriving;
                    continue;
                }

                // ---- Turn toward target ----
                let desired_heading = SimFloat::atan2(delta.z, delta.x);
                let angle_diff = wrap_angle(desired_heading - heading);

                let new_heading = if angle_diff.abs() <= params.turn_rate {
                    desired_heading
                } else if angle_diff > SimFloat::ZERO {
                    wrap_angle(heading + params.turn_rate)
                } else {
                    wrap_angle(heading - params.turn_rate)
                };
                world.get_mut::<Heading>(entity).unwrap().angle = new_heading;

                // ---- Accelerate along current heading ----
                let dir = SimVec3::new(new_heading.cos(), SimFloat::ZERO, new_heading.sin());

                let cur_vel = world.get::<Velocity>(entity).unwrap().vel;
                let cur_speed = cur_vel.length();
                let new_speed = (cur_speed + params.acceleration).min(params.max_speed);

                let new_vel = dir * new_speed;
                world.get_mut::<Velocity>(entity).unwrap().vel = new_vel;

                // ---- Integrate position ----
                let new_vel = world.get::<Velocity>(entity).unwrap().vel;
                world.get_mut::<Position>(entity).unwrap().pos = pos + new_vel;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/movement_tests.rs"]
mod tests;
