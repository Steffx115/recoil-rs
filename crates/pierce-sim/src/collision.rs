//! Unit-unit collision detection and response.
//!
//! Uses the [`SpatialGrid`] for broad-phase neighbour queries, then does
//! circle-circle overlap tests in the narrow phase.  Overlapping pairs are
//! pushed apart equally along the line between their centres.
//!
//! All math uses deterministic [`SimFloat`] fixed-point arithmetic.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;

use crate::components::{CollisionRadius, MoveState, Position};
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2, SimVec3};

/// Run one tick of unit-unit collision detection and response.
///
/// For every entity that has both a [`Position`] and a [`CollisionRadius`]:
/// 1. **Broad phase** — query the [`SpatialGrid`] for neighbours within the
///    maximum possible overlap distance (sum of the two largest radii would
///    be ideal, but we conservatively use `entity_radius * 2 + max_other_radius`
///    — here we just use the entity's own radius plus a generous search range
///    by querying with each entity's radius doubled, which may pull in a few
///    extra candidates but is cheap to reject).
/// 2. **Narrow phase** — circle-circle overlap: `distance < r_a + r_b`.
/// 3. **Response** — push both entities apart by half the overlap distance
///    along the line connecting their centres.
///
/// Each pair is processed exactly once by only handling pairs where
/// `entity_a.to_bits() < neighbour.to_bits()`, eliminating the need for a
/// `BTreeSet` of processed pairs.
pub fn collision_system(world: &mut World) {
    // We need the spatial grid to exist as a resource.
    let has_grid = world.get_resource::<SpatialGrid>().is_some();
    if !has_grid {
        return;
    }

    // Collect all collidable entities and their current state.
    // `is_mobile` tracks whether the entity has MoveState (can be pushed).
    let entities: Vec<(Entity, SimVec3, SimFloat, bool)> = world
        .query_filtered::<(Entity, &Position, &CollisionRadius, Option<&MoveState>), ()>()
        .iter(world)
        .map(|(e, p, r, ms)| (e, p.pos, r.radius, ms.is_some()))
        .collect();

    // Find the maximum radius across all entities so we can do a single
    // broad-phase query radius = own_radius + max_other_radius.
    let max_radius = entities
        .iter()
        .map(|&(_, _, r, _)| r)
        .max()
        .unwrap_or(SimFloat::ZERO);

    // Accumulate displacement vectors so we can apply them all at once
    // (avoids order-dependent position changes within the same tick).
    let mut displacements: Vec<(Entity, SimVec3)> = Vec::new();

    let grid = world.resource::<SpatialGrid>();

    // Build a lookup for mobility.
    let mobility: std::collections::HashMap<Entity, bool> = entities
        .iter()
        .map(|&(e, _, _, mobile)| (e, mobile))
        .collect();

    for &(entity_a, pos_a, radius_a, _) in &entities {
        let search_radius = radius_a + max_radius;
        let center = SimVec2::new(pos_a.x, pos_a.z);
        let neighbours = grid.units_in_radius(center, search_radius);

        let bits_a = entity_a.to_bits();

        for neighbour in neighbours {
            // Only process each pair once: require entity_a < neighbour by bits.
            // This replaces the BTreeSet<(u64,u64)> approach with an O(1) check.
            if bits_a >= neighbour.to_bits() {
                continue;
            }

            // Look up neighbour data.
            let (pos_b, radius_b) = match (
                world.get::<Position>(neighbour),
                world.get::<CollisionRadius>(neighbour),
            ) {
                (Some(p), Some(r)) => (p.pos, r.radius),
                _ => continue,
            };

            // Narrow phase: circle-circle overlap test.
            let delta = pos_b - pos_a;
            let dist_sq = delta.length_squared();
            let sum_radii = radius_a + radius_b;
            let sum_radii_sq = sum_radii * sum_radii;

            if dist_sq >= sum_radii_sq {
                continue; // no overlap
            }

            let dist = dist_sq.sqrt();

            // Overlap amount.
            let overlap = sum_radii - dist;
            let half_overlap = overlap / SimFloat::TWO;

            // Direction from A to B; if centres coincide, push along +X.
            let direction = if dist > SimFloat::ZERO {
                delta / dist
            } else {
                SimVec3::new(SimFloat::ONE, SimFloat::ZERO, SimFloat::ZERO)
            };

            // Check mobility: entities without MoveState (buildings) are immovable.
            let a_mobile = mobility.get(&entity_a).copied().unwrap_or(true);
            let b_mobile = mobility.get(&neighbour).copied().unwrap_or(true);

            match (a_mobile, b_mobile) {
                (true, true) => {
                    // Both mobile: split displacement evenly.
                    displacements.push((entity_a, direction * (-half_overlap)));
                    displacements.push((neighbour, direction * half_overlap));
                }
                (true, false) => {
                    // Only A is mobile: A gets full push away from building B.
                    displacements.push((entity_a, direction * (-overlap)));
                }
                (false, true) => {
                    // Only B is mobile: B gets full push away from building A.
                    displacements.push((neighbour, direction * overlap));
                }
                (false, false) => {
                    // Both immobile: no displacement.
                }
            }
        }
    }

    // Apply accumulated displacements.
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
#[path = "collision_tests.rs"]
mod tests;
