//! Unit-unit collision detection and response.
//!
//! Uses the [`SpatialGrid`] for broad-phase neighbour queries, then does
//! circle-circle overlap tests in the narrow phase.  Overlapping pairs are
//! pushed apart equally along the line between their centres.
//!
//! All math uses deterministic [`SimFloat`] fixed-point arithmetic.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;

use crate::components::{CollisionRadius, MoveState, Position};
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2, SimVec3};

/// Pre-collected collision data for one entity.
#[derive(Clone, Copy)]
struct CollisionEntity {
    entity: Entity,
    pos: SimVec3,
    radius: SimFloat,
    is_mobile: bool,
}

/// Run one tick of unit-unit collision detection and response.
pub fn collision_system(world: &mut World) {
    let has_grid = world.get_resource::<SpatialGrid>().is_some();
    if !has_grid {
        return;
    }

    // Pre-collect all collision data in one pass.
    let entities: Vec<CollisionEntity> = world
        .query_filtered::<(Entity, &Position, &CollisionRadius, Option<&MoveState>), ()>()
        .iter(world)
        .map(|(e, p, r, ms)| CollisionEntity {
            entity: e,
            pos: p.pos,
            radius: r.radius,
            is_mobile: ms.is_some(),
        })
        .collect();

    if entities.is_empty() {
        return;
    }

    // Build lookup by entity bits for O(1) neighbour data access.
    // Uses BTreeMap for determinism (no HashMap in sim code).
    let lookup: BTreeMap<u64, usize> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| (e.entity.to_bits(), i))
        .collect();

    let max_radius = entities
        .iter()
        .map(|e| e.radius)
        .max()
        .unwrap_or(SimFloat::ZERO);

    let grid = world.resource::<SpatialGrid>();

    // Accumulate displacement vectors.
    let mut displacements: Vec<(Entity, SimVec3)> = Vec::new();

    for ce in &entities {
        let search_radius = ce.radius + max_radius;
        let center = SimVec2::new(ce.pos.x, ce.pos.z);
        let neighbours = grid.units_in_radius(center, search_radius);

        let bits_a = ce.entity.to_bits();

        for neighbour in neighbours {
            let bits_b = neighbour.to_bits();
            if bits_a >= bits_b {
                continue;
            }

            // Look up neighbour from pre-collected data (no ECS access).
            let Some(&idx_b) = lookup.get(&bits_b) else {
                continue;
            };
            let nb = &entities[idx_b];

            // Narrow phase: circle-circle overlap.
            let dx = nb.pos.x - ce.pos.x;
            let dz = nb.pos.z - ce.pos.z;
            let dist_sq = dx * dx + dz * dz;
            let sum_radii = ce.radius + nb.radius;
            let sum_radii_sq = sum_radii * sum_radii;

            if dist_sq >= sum_radii_sq {
                continue;
            }

            // Push apart using unnormalized delta — avoids sqrt and div entirely.
            // Scale: push = delta * (sum_radii_sq - dist_sq) / (4 * sum_radii_sq)
            // This approximates half-overlap along the direction. The factor
            // naturally goes to zero as overlap vanishes and increases for deep
            // overlaps. Multiply is cheap; division by power-of-2 is a shift.
            //
            // Simplified: push = delta * overlap_fraction / 2
            // where overlap_fraction = (sum_radii_sq - dist_sq) / sum_radii_sq
            // But division is expensive. Use a simpler approach:
            // push each entity by delta * PUSH_FACTOR when overlapping.
            // PUSH_FACTOR = 1/4 gives stable separation over a few frames.
            let push_scale = SimFloat::from_ratio(1, 4);

            let (push_x, push_z) = if dist_sq > SimFloat::ZERO {
                (dx * push_scale, dz * push_scale)
            } else {
                // Coincident: push along +X by radius.
                (ce.radius * push_scale, SimFloat::ZERO)
            };

            match (ce.is_mobile, nb.is_mobile) {
                (true, true) => {
                    displacements.push((
                        ce.entity,
                        SimVec3::new(-push_x, SimFloat::ZERO, -push_z),
                    ));
                    displacements.push((
                        nb.entity,
                        SimVec3::new(push_x, SimFloat::ZERO, push_z),
                    ));
                }
                (true, false) => {
                    let two = SimFloat::TWO;
                    displacements.push((
                        ce.entity,
                        SimVec3::new(-push_x * two, SimFloat::ZERO, -push_z * two),
                    ));
                }
                (false, true) => {
                    let two = SimFloat::TWO;
                    displacements.push((
                        nb.entity,
                        SimVec3::new(push_x * two, SimFloat::ZERO, push_z * two),
                    ));
                }
                (false, false) => {}
            }
        }
    }

    // Apply displacements.
    for (entity, disp) in displacements {
        if let Some(mut pos) = world.get_mut::<Position>(entity) {
            pos.pos += disp;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/collision_tests.rs"]
mod tests;
