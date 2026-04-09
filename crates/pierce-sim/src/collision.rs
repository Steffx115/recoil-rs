//! Unit-unit collision detection and response.
//!
//! Uses the [`SpatialGrid`] for broad-phase neighbour queries, then does
//! circle-circle overlap tests in the narrow phase.  Overlapping pairs are
//! pushed apart along the line between their centres.
//!
//! All math uses deterministic [`SimFloat`] fixed-point arithmetic.
//! Collision pairs are processed in parallel via rayon. Determinism is
//! guaranteed because each pair produces a displacement keyed by entity,
//! and displacements are applied in sorted entity order.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

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

    // Pre-collect all collision data in one query pass.
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

    // Build lookup by entity bits for neighbour data access.
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

    let grid = world.resource::<SpatialGrid>().clone();

    // Parallel: each entity finds its collision pairs and produces displacements.
    let push_scale = SimFloat::from_ratio(1, 4);
    let two = SimFloat::TWO;

    let all_displacements: Vec<Vec<(u64, SimVec3)>> = entities
        .par_iter()
        .map(|ce| {
            let mut local_disps: Vec<(u64, SimVec3)> = Vec::new();
            let search_radius = ce.radius + max_radius;
            let center = SimVec2::new(ce.pos.x, ce.pos.z);
            let neighbours = grid.units_in_radius(center, search_radius);

            let bits_a = ce.entity.to_bits();

            for neighbour in neighbours {
                let bits_b = neighbour.to_bits();
                if bits_a >= bits_b {
                    continue;
                }

                let Some(&idx_b) = lookup.get(&bits_b) else {
                    continue;
                };
                let nb = &entities[idx_b];

                let dx = nb.pos.x - ce.pos.x;
                let dz = nb.pos.z - ce.pos.z;
                let dist_sq = dx * dx + dz * dz;
                let sum_radii = ce.radius + nb.radius;
                let sum_radii_sq = sum_radii * sum_radii;

                if dist_sq >= sum_radii_sq {
                    continue;
                }

                let (push_x, push_z) = if dist_sq > SimFloat::ZERO {
                    (dx * push_scale, dz * push_scale)
                } else {
                    (ce.radius * push_scale, SimFloat::ZERO)
                };

                match (ce.is_mobile, nb.is_mobile) {
                    (true, true) => {
                        local_disps.push((
                            bits_a,
                            SimVec3::new(-push_x, SimFloat::ZERO, -push_z),
                        ));
                        local_disps.push((
                            bits_b,
                            SimVec3::new(push_x, SimFloat::ZERO, push_z),
                        ));
                    }
                    (true, false) => {
                        local_disps.push((
                            bits_a,
                            SimVec3::new(-push_x * two, SimFloat::ZERO, -push_z * two),
                        ));
                    }
                    (false, true) => {
                        local_disps.push((
                            bits_b,
                            SimVec3::new(push_x * two, SimFloat::ZERO, push_z * two),
                        ));
                    }
                    (false, false) => {}
                }
            }
            local_disps
        })
        .collect();

    // Merge displacements deterministically: accumulate per entity in sorted order.
    let mut merged: BTreeMap<u64, SimVec3> = BTreeMap::new();
    for disps in &all_displacements {
        for &(bits, disp) in disps {
            merged
                .entry(bits)
                .and_modify(|d| *d += disp)
                .or_insert(disp);
        }
    }

    // Apply in deterministic (BTreeMap sorted) order.
    for (bits, disp) in &merged {
        let entity = Entity::from_bits(*bits);
        if let Some(mut pos) = world.get_mut::<Position>(entity) {
            pos.pos += *disp;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/collision_tests.rs"]
mod tests;
