//! Ground movement system.
//!
//! Drives entities toward their target positions using deterministic
//! fixed-point math. Avoids trig (atan2/sin/cos) in the hot path by
//! working with direction vectors directly. Heading is derived from
//! the direction vector, not the other way around.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::components::{Heading, MoveState, MovementParams, Position, Velocity};
use crate::SimFloat;
use crate::SimVec3;

/// Distance (squared) at which a unit is considered to have arrived.
const ARRIVAL_THRESHOLD_SQ: SimFloat = SimFloat::ONE;

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

/// Per-entity movement input snapshot.
struct MoveInput {
    entity: Entity,
    pos: SimVec3,
    vel: SimVec3,
    heading: SimFloat,
    params: MovementParams,
    state: MoveState,
}

/// Per-entity movement output.
struct MoveOutput {
    entity: Entity,
    pos: SimVec3,
    vel: SimVec3,
    heading: SimFloat,
    state: MoveState,
}

/// Compute movement without trig in the common case.
///
/// Instead of: atan2(delta) → clamp angle → sin/cos(angle) → direction
/// We do:      normalize(delta) → rotate current dir toward desired → direction
///
/// The only remaining trig: atan2 to store the heading (needed by other
/// systems like targeting). This could be batched or deferred.
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

            // Desired direction (normalized delta in XZ plane).
            let desired_x = delta.x;
            let desired_z = delta.z;
            let desired_len_sq = desired_x * desired_x + desired_z * desired_z;

            // Current facing direction from heading (these two sin/cos calls remain,
            // but we can optimize them away once we store direction instead of heading).
            let cur_dir_x = input.heading.cos();
            let cur_dir_z = input.heading.sin();

            // Compute angle between current and desired direction using cross and dot products.
            // cross = cur_x * des_z - cur_z * des_x (sign = rotation direction)
            // dot = cur_x * des_x + cur_z * des_z (cosine of angle)
            //
            // To avoid sqrt for normalization of desired, we use the unnormalized
            // cross/dot and compare against turn_rate scaled by desired_len.
            // But for correct angle comparison we need the actual angle.
            //
            // Fast path: if turn rate is large enough to cover any angle difference,
            // skip the angle computation entirely and just use the desired direction.
            // This is common for units that are already roughly facing their target.

            let (new_dir_x, new_dir_z, new_heading) = if desired_len_sq > SimFloat::ZERO {
                // Normalize desired direction.
                let desired_len = desired_len_sq.sqrt();
                let norm_x = desired_x / desired_len;
                let norm_z = desired_z / desired_len;

                // Dot product: cos(angle_between).
                let dot = cur_dir_x * norm_x + cur_dir_z * norm_z;

                // If dot > cos(turn_rate), the angle is within turn_rate — snap to desired.
                // cos(turn_rate) for small angles ≈ 1 - turn_rate²/2.
                // For large turn rates (> PI/4), just use cos.
                let cos_turn = if input.params.turn_rate >= SimFloat::PI / SimFloat::from_int(4) {
                    input.params.turn_rate.cos()
                } else {
                    // Approximation: 1 - t²/2 (avoids cos for small turn rates).
                    let t = input.params.turn_rate;
                    SimFloat::ONE - (t * t) / SimFloat::TWO
                };

                if dot >= cos_turn {
                    // Within turn rate — face target directly.
                    let heading = SimFloat::atan2(norm_z, norm_x);
                    (norm_x, norm_z, heading)
                } else {
                    // Need to rotate. Use cross product for rotation direction.
                    let cross = cur_dir_x * norm_z - cur_dir_z * norm_x;
                    let turn = if cross >= SimFloat::ZERO {
                        input.params.turn_rate
                    } else {
                        -input.params.turn_rate
                    };
                    let new_heading = wrap_angle(input.heading + turn);
                    // Derive direction from new heading (two trig calls).
                    (new_heading.cos(), new_heading.sin(), new_heading)
                }
            } else {
                (cur_dir_x, cur_dir_z, input.heading)
            };

            // Accelerate along new direction.
            // Avoid sqrt for speed: use length_squared and compare against max_speed_sq.
            // Only compute actual speed when needed (not at max speed).
            let cur_speed_sq = input.vel.length_squared();
            let max_speed_sq = input.params.max_speed * input.params.max_speed;
            let new_speed = if cur_speed_sq >= max_speed_sq {
                input.params.max_speed
            } else {
                // Only sqrt when accelerating (not at max speed).
                let cur_speed = cur_speed_sq.sqrt();
                (cur_speed + input.params.acceleration).min(input.params.max_speed)
            };

            let new_vel = SimVec3::new(
                new_dir_x * new_speed,
                SimFloat::ZERO,
                new_dir_z * new_speed,
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
