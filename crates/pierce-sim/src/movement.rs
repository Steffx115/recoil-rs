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
mod tests {
    use super::*;
    use crate::components::*;

    /// Helper: spawn a unit with movement components and return its entity id.
    fn spawn_unit(
        world: &mut World,
        pos: SimVec3,
        heading: SimFloat,
        state: MoveState,
        max_speed: SimFloat,
        acceleration: SimFloat,
        turn_rate: SimFloat,
    ) -> Entity {
        world
            .spawn((
                Position { pos },
                Velocity { vel: SimVec3::ZERO },
                Heading { angle: heading },
                state,
                MovementParams {
                    max_speed,
                    acceleration,
                    turn_rate,
                },
            ))
            .id()
    }

    // ---- Idle unit stays put ----

    #[test]
    fn idle_unit_stays_put() {
        let mut world = World::new();
        let e = spawn_unit(
            &mut world,
            SimVec3::ZERO,
            SimFloat::ZERO,
            MoveState::Idle,
            SimFloat::ONE,
            SimFloat::ONE,
            SimFloat::PI,
        );

        movement_system(&mut world);

        let pos = world.get::<Position>(e).unwrap().pos;
        assert_eq!(pos, SimVec3::ZERO);
        let vel = world.get::<Velocity>(e).unwrap().vel;
        assert_eq!(vel, SimVec3::ZERO);
    }

    // ---- Unit moves from A to B and arrives ----

    #[test]
    fn unit_moves_to_target_and_arrives() {
        let mut world = World::new();
        let start = SimVec3::ZERO;
        let target = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);

        let e = spawn_unit(
            &mut world,
            start,
            SimFloat::ZERO, // already facing +X
            MoveState::MovingTo(target),
            SimFloat::ONE, // max_speed = 1
            SimFloat::ONE, // instant acceleration
            SimFloat::PI,  // can turn instantly
        );

        // Run enough ticks to reach the target (10 units at speed 1)
        for _ in 0..20 {
            movement_system(&mut world);
        }

        let state = world.get::<MoveState>(e).unwrap();
        assert_eq!(
            *state,
            MoveState::Idle,
            "unit should have arrived and gone idle"
        );

        let pos = world.get::<Position>(e).unwrap().pos;
        let dist = pos.distance(target);
        assert!(
            dist <= SimFloat::ONE,
            "unit should be at or near the target, dist = {}",
            dist.to_f64()
        );
    }

    // ---- Unit turns before moving ----

    #[test]
    fn unit_turns_toward_target() {
        let mut world = World::new();
        // Target is along +Z (heading = PI/2), but unit starts facing +X (heading = 0).
        let target = SimVec3::new(SimFloat::ZERO, SimFloat::ZERO, SimFloat::from_int(50));
        let slow_turn = SimFloat::from_ratio(1, 10); // 0.1 rad/tick

        let e = spawn_unit(
            &mut world,
            SimVec3::ZERO,
            SimFloat::ZERO,
            MoveState::MovingTo(target),
            SimFloat::ONE,
            SimFloat::ONE,
            slow_turn,
        );

        // After one tick the heading should have changed by ~turn_rate
        movement_system(&mut world);

        let heading = world.get::<Heading>(e).unwrap().angle;
        // The desired heading is atan2(z, x) = atan2(50, 0) = PI/2 ≈ 1.57
        // With a turn rate of 0.1, after one tick heading should be ~0.1
        let expected = slow_turn;
        let diff = (heading - expected).abs();
        assert!(
            diff < SimFloat::from_ratio(1, 100),
            "heading should be roughly turn_rate after one tick, got {}",
            heading.to_f64()
        );
    }

    // ---- Determinism: same inputs produce same outputs ----

    #[test]
    fn determinism_same_inputs_same_outputs() {
        fn run_sim() -> (SimVec3, SimFloat) {
            let mut world = World::new();
            let target = SimVec3::new(SimFloat::from_int(5), SimFloat::ZERO, SimFloat::from_int(5));
            let e = spawn_unit(
                &mut world,
                SimVec3::ZERO,
                SimFloat::ZERO,
                MoveState::MovingTo(target),
                SimFloat::from_ratio(3, 2), // 1.5
                SimFloat::HALF,
                SimFloat::from_ratio(1, 4),
            );

            for _ in 0..30 {
                movement_system(&mut world);
            }

            let pos = world.get::<Position>(e).unwrap().pos;
            let heading = world.get::<Heading>(e).unwrap().angle;
            (pos, heading)
        }

        let (pos_a, heading_a) = run_sim();
        let (pos_b, heading_b) = run_sim();

        assert_eq!(pos_a, pos_b, "positions must be bit-identical across runs");
        assert_eq!(
            heading_a, heading_b,
            "headings must be bit-identical across runs"
        );
    }

    // ---- Arriving transitions to Idle ----

    #[test]
    fn arriving_transitions_to_idle() {
        let mut world = World::new();
        let e = spawn_unit(
            &mut world,
            SimVec3::ZERO,
            SimFloat::ZERO,
            MoveState::Arriving,
            SimFloat::ONE,
            SimFloat::ONE,
            SimFloat::PI,
        );

        movement_system(&mut world);

        let state = world.get::<MoveState>(e).unwrap();
        assert_eq!(*state, MoveState::Idle);
        let vel = world.get::<Velocity>(e).unwrap().vel;
        assert_eq!(vel, SimVec3::ZERO);
    }
}
