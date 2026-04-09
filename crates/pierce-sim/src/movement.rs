//! Ground movement system.
//!
//! Drives entities toward their target positions using deterministic
//! fixed-point math. When `BatchMathBackend` is available, expensive
//! operations (atan2, sin/cos) are batched for CPU/GPU parallelism.
//! Falls back to per-entity rayon parallel when no backend is present.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::components::{Heading, MoveState, MovementParams, Position, Velocity};
use crate::compute::BatchMathBackend;
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

/// Run one tick of the ground movement system.
pub fn movement_system(world: &mut World) {
    // Collect all movable entities.
    let inputs: Vec<(Entity, SimVec3, SimVec3, SimFloat, MovementParams, MoveState)> = world
        .query::<(
            Entity,
            &Position,
            &Velocity,
            &Heading,
            &MoveState,
            &MovementParams,
        )>()
        .iter(world)
        .map(|(e, p, v, h, ms, mp)| (e, p.pos, v.vel, h.angle, mp.clone(), ms.clone()))
        .collect();

    if inputs.is_empty() {
        return;
    }

    // Try batched path if backend available.
    let has_backend = world.contains_resource::<BatchMathBackend>();
    if has_backend {
        movement_batched(world, &inputs);
    } else {
        movement_scalar(world, &inputs);
    }
}

/// Batched movement: gather SoA arrays, batch atan2 + sincos, apply results.
fn movement_batched(
    world: &mut World,
    inputs: &[(Entity, SimVec3, SimVec3, SimFloat, MovementParams, MoveState)],
) {
    // Separate moving entities from idle/arriving.
    // For idle/arriving, just zero velocity and transition.
    struct MovingEntity {
        idx: usize, // index into inputs
        target: SimVec3,
    }

    let mut idle_outputs: Vec<(Entity, SimVec3, SimFloat, MoveState)> = Vec::new();
    let mut moving: Vec<MovingEntity> = Vec::new();

    for (i, (entity, pos, _vel, heading, _params, state)) in inputs.iter().enumerate() {
        match state {
            MoveState::Idle => {
                idle_outputs.push((*entity, SimVec3::ZERO, *heading, MoveState::Idle));
            }
            MoveState::Arriving => {
                idle_outputs.push((*entity, SimVec3::ZERO, *heading, MoveState::Idle));
            }
            MoveState::MovingTo(target) => {
                // Quick arrival check (cheap, no batch needed).
                let delta = *target - *pos;
                let dist_sq = delta.length_squared();
                if dist_sq <= ARRIVAL_THRESHOLD_SQ {
                    idle_outputs.push((*entity, SimVec3::ZERO, *heading, MoveState::Arriving));
                } else {
                    moving.push(MovingEntity { idx: i, target: *target });
                }
            }
        }
    }

    // Apply idle/arrived outputs.
    for (entity, vel, heading, state) in &idle_outputs {
        if let Some(mut v) = world.get_mut::<Velocity>(*entity) {
            v.vel = *vel;
        }
        if matches!(state, MoveState::Arriving) {
            // Snap to target position.
        }
        if let Some(mut ms) = world.get_mut::<MoveState>(*entity) {
            *ms = state.clone();
        }
    }

    if moving.is_empty() {
        return;
    }

    // Build SoA arrays for batch heading (atan2).
    let n = moving.len();
    let mut dx = Vec::with_capacity(n);
    let mut dz = Vec::with_capacity(n);

    for m in &moving {
        let (_, pos, _, _, _, _) = &inputs[m.idx];
        let delta_x = m.target.x - pos.x;
        let delta_z = m.target.z - pos.z;
        dx.push(delta_x.raw());
        dz.push(delta_z.raw());
    }

    // Batch atan2.
    let mut backend = world.remove_resource::<BatchMathBackend>().unwrap();
    let desired_headings = backend.ops.batch_heading(&dx, &dz);

    // Apply turn rate clamping (cheap, CPU-side).
    let mut clamped_headings = Vec::with_capacity(n);
    for (i, m) in moving.iter().enumerate() {
        let (_, _, _, heading, params, _) = &inputs[m.idx];
        let desired = SimFloat::from_raw(desired_headings[i]);
        let angle_diff = wrap_angle(desired - *heading);

        let new_heading = if angle_diff.abs() <= params.turn_rate {
            desired
        } else if angle_diff > SimFloat::ZERO {
            wrap_angle(*heading + params.turn_rate)
        } else {
            wrap_angle(*heading - params.turn_rate)
        };
        clamped_headings.push(new_heading.raw());
    }

    // Batch sin/cos for direction vectors.
    let (sin_vals, cos_vals) = backend.ops.batch_sincos(&clamped_headings);

    world.insert_resource(backend);

    // Compute velocities and positions (cheap scalar ops).
    for (i, m) in moving.iter().enumerate() {
        let (entity, pos, vel, _, params, state) = &inputs[m.idx];

        let dir_x = SimFloat::from_raw(cos_vals[i]);
        let dir_z = SimFloat::from_raw(sin_vals[i]);

        let cur_speed = vel.length();
        let new_speed = (cur_speed + params.acceleration).min(params.max_speed);

        let new_vel = SimVec3::new(
            dir_x * new_speed,
            SimFloat::ZERO,
            dir_z * new_speed,
        );
        let new_pos = *pos + new_vel;
        let new_heading = SimFloat::from_raw(clamped_headings[i]);

        if let Some(mut p) = world.get_mut::<Position>(*entity) {
            p.pos = new_pos;
        }
        if let Some(mut v) = world.get_mut::<Velocity>(*entity) {
            v.vel = new_vel;
        }
        if let Some(mut h) = world.get_mut::<Heading>(*entity) {
            h.angle = new_heading;
        }
        // MoveState stays MovingTo(target) — no change needed.
    }
}

/// Scalar per-entity movement via rayon (fallback when no batch backend).
fn movement_scalar(
    world: &mut World,
    inputs: &[(Entity, SimVec3, SimVec3, SimFloat, MovementParams, MoveState)],
) {
    struct MoveOutput {
        entity: Entity,
        pos: SimVec3,
        vel: SimVec3,
        heading: SimFloat,
        state: MoveState,
    }

    let outputs: Vec<MoveOutput> = inputs
        .par_iter()
        .map(|(entity, pos, vel, heading, params, state)| match state {
            MoveState::Idle => MoveOutput {
                entity: *entity,
                pos: *pos,
                vel: SimVec3::ZERO,
                heading: *heading,
                state: MoveState::Idle,
            },
            MoveState::Arriving => MoveOutput {
                entity: *entity,
                pos: *pos,
                vel: SimVec3::ZERO,
                heading: *heading,
                state: MoveState::Idle,
            },
            MoveState::MovingTo(target) => {
                let delta = *target - *pos;
                let dist_sq = delta.length_squared();

                if dist_sq <= ARRIVAL_THRESHOLD_SQ {
                    return MoveOutput {
                        entity: *entity,
                        pos: *target,
                        vel: SimVec3::ZERO,
                        heading: *heading,
                        state: MoveState::Arriving,
                    };
                }

                let desired_heading = SimFloat::atan2(delta.z, delta.x);
                let angle_diff = wrap_angle(desired_heading - *heading);

                let new_heading = if angle_diff.abs() <= params.turn_rate {
                    desired_heading
                } else if angle_diff > SimFloat::ZERO {
                    wrap_angle(*heading + params.turn_rate)
                } else {
                    wrap_angle(*heading - params.turn_rate)
                };

                let dir = SimVec3::new(new_heading.cos(), SimFloat::ZERO, new_heading.sin());
                let cur_speed = vel.length();
                let new_speed = (cur_speed + params.acceleration).min(params.max_speed);
                let new_vel = dir * new_speed;
                let new_pos = *pos + new_vel;

                MoveOutput {
                    entity: *entity,
                    pos: new_pos,
                    vel: new_vel,
                    heading: new_heading,
                    state: MoveState::MovingTo(*target),
                }
            }
        })
        .collect();

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
