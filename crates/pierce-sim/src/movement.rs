//! Ground movement system.
//!
//! Drives entities that have a [`MoveState`] toward their target position,
//! respecting per-entity [`MovementParams`] (max speed, acceleration, turn
//! rate).  All math uses deterministic [`SimFloat`] fixed-point arithmetic.
//!
//! Movement updates are computed in parallel via rayon, then applied in
//! deterministic entity order.

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

/// Snapshot of a movable entity's state before computation.
#[derive(Clone)]
struct MoveInput {
    entity: Entity,
    pos: SimVec3,
    vel: SimVec3,
    heading: SimFloat,
    params: MovementParams,
    state: MoveState,
}

/// Computed result for one entity.
struct MoveOutput {
    entity: Entity,
    pos: SimVec3,
    vel: SimVec3,
    heading: SimFloat,
    state: MoveState,
}

/// Compute one entity's movement update (pure function, no ECS access).
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

            let desired_heading = SimFloat::atan2(delta.z, delta.x);
            let angle_diff = wrap_angle(desired_heading - input.heading);

            let new_heading = if angle_diff.abs() <= input.params.turn_rate {
                desired_heading
            } else if angle_diff > SimFloat::ZERO {
                wrap_angle(input.heading + input.params.turn_rate)
            } else {
                wrap_angle(input.heading - input.params.turn_rate)
            };

            let dir = SimVec3::new(new_heading.cos(), SimFloat::ZERO, new_heading.sin());
            let cur_speed = input.vel.length();
            let new_speed = (cur_speed + input.params.acceleration).min(input.params.max_speed);
            let new_vel = dir * new_speed;
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
    // 1. Collect all movable entities' state in one query pass.
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

    // 2. Compute all updates in parallel. Each entity is independent.
    let outputs: Vec<MoveOutput> = inputs.par_iter().map(compute_movement).collect();

    // 3. Write back results (sequential, deterministic).
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
