//! Ground movement system.
//!
//! Drives entities toward their target positions using deterministic
//! fixed-point math. Avoids trig (atan2/sin/cos) in the hot path by
//! working with direction vectors directly. Heading is derived from
//! the direction vector, not the other way around.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use pierce_math::Angle;

use crate::components::{Heading, MoveState, MovementParams, Position, Velocity};
use crate::SimFloat;
use crate::SimVec3;

/// Distance (squared) at which a unit is considered to have arrived.
const ARRIVAL_THRESHOLD_SQ: SimFloat = SimFloat::ONE;


/// Per-entity movement input snapshot.
struct MoveInput {
    entity: Entity,
    pos: SimVec3,
    vel: SimVec3,
    heading: Angle,
    params: MovementParams,
    state: MoveState,
}

/// Per-entity movement output.
struct MoveOutput {
    entity: Entity,
    pos: SimVec3,
    vel: SimVec3,
    heading: Angle,
    state: MoveState,
}

/// Compute movement using Angle for heading. No atan2 in the common case.
fn compute_movement(input: &MoveInput) -> MoveOutput {
    match input.state {
        MoveState::Idle => MoveOutput {
            entity: input.entity,
            pos: input.pos,
            vel: SimVec3::ZERO,
            heading: input.heading,
            state: MoveState::Idle,
        },
        MoveState::Arriving => MoveOutput {
            entity: input.entity,
            pos: input.pos,
            vel: SimVec3::ZERO,
            heading: input.heading,
            state: MoveState::Idle,
        },
        MoveState::MovingTo(target) => {
            let delta = target - input.pos;
            let dist_sq = delta.length_squared();

            if dist_sq <= ARRIVAL_THRESHOLD_SQ {
                return MoveOutput {
                    entity: input.entity,
                    pos: target,
                    vel: SimVec3::ZERO,
                    heading: input.heading,
                    state: MoveState::Arriving,
                };
            }

            // Desired heading via Angle::atan2 (uses hardware atan2 via libm).
            let desired = Angle::atan2(delta.z, delta.x);

            // Turn rate as Angle. Convert SimFloat radians → Angle.
            let turn_rate_angle = Angle::from_radians(input.params.turn_rate);

            // Signed difference: how far to turn.
            let diff = input.heading.signed_diff(desired);
            let abs_diff = diff.unsigned_abs();

            let new_heading = if abs_diff <= turn_rate_angle.0 {
                // Within turn rate — snap to desired.
                desired
            } else if diff > 0 {
                input.heading + turn_rate_angle
            } else {
                input.heading - turn_rate_angle
            };

            // Direction from heading via LUT sin/cos (fast table lookup).
            let (dir_z, dir_x) = new_heading.sincos();

            // Speed: avoid sqrt when at max speed.
            let cur_speed_sq = input.vel.length_squared();
            let max_speed_sq = input.params.max_speed * input.params.max_speed;
            let new_speed = if cur_speed_sq >= max_speed_sq {
                input.params.max_speed
            } else {
                let cur_speed = cur_speed_sq.sqrt();
                (cur_speed + input.params.acceleration).min(input.params.max_speed)
            };

            let new_vel = SimVec3::new(
                dir_x * new_speed,
                SimFloat::ZERO,
                dir_z * new_speed,
            );
            let new_pos = input.pos + new_vel;

            MoveOutput {
                entity: input.entity,
                pos: new_pos,
                vel: new_vel,
                heading: new_heading,
                state: MoveState::MovingTo(target),
            }
        }
    }
}

/// Run one tick of the ground movement system.
pub fn movement_system(world: &mut World) {
    let inputs: Vec<MoveInput> = world
        .query::<(
            Entity,
            &Position,
            &Velocity,
            &Heading,
            &MoveState,
            &MovementParams,
        )>()
        .iter(world)
        .map(|(e, p, v, h, ms, mp)| MoveInput {
            entity: e,
            pos: p.pos,
            vel: v.vel,
            heading: h.angle,
            params: mp.clone(),
            state: ms.clone(),
        })
        .collect();

    if inputs.is_empty() {
        return;
    }

    let outputs: Vec<MoveOutput> = inputs.par_iter().map(compute_movement).collect();

    for out in outputs {
        if let Some(mut pos) = world.get_mut::<Position>(out.entity) {
            pos.pos = out.pos;
        }
        if let Some(mut vel) = world.get_mut::<Velocity>(out.entity) {
            vel.vel = out.vel;
        }
        if let Some(mut heading) = world.get_mut::<Heading>(out.entity) {
            heading.angle = out.heading;
        }
        if let Some(mut ms) = world.get_mut::<MoveState>(out.entity) {
            *ms = out.state;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/movement_tests.rs"]
mod tests;
