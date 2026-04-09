//! Unit-unit collision detection and response.
//!
//! Uses pre-collected `SimFrameData` and the `SpatialGrid` for broad-phase
//! neighbour queries. Narrow phase does circle-circle overlap. Overlapping
//! pairs are pushed apart. All math uses deterministic `SimFloat`.
//! Collision pairs are processed in parallel via rayon.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::components::Position;
use crate::frame_data::SimFrameData;
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2, SimVec3};

/// Run one tick of unit-unit collision detection and response.
/// Uses pre-collected `SimFrameData` if available, otherwise queries ECS.
pub fn collision_system(world: &mut World) {
    let has_grid = world.get_resource::<SpatialGrid>().is_some();
    if !has_grid {
        return;
    }

    // Use pre-collected frame data if available, else collect inline.
    let entities: Vec<crate::frame_data::CollisionData> =
        if let Some(frame) = world.get_resource::<SimFrameData>() {
            frame.collision_entities.clone()
        } else {
            use crate::components::{CollisionRadius, MoveState};
            world
                .query_filtered::<(Entity, &Position, &CollisionRadius, Option<&MoveState>), ()>()
                .iter(world)
                .map(|(e, p, r, ms)| crate::frame_data::CollisionData {
                    entity: e,
                    bits: e.to_bits(),
                    pos_x: p.pos.x,
                    pos_z: p.pos.z,
                    radius: r.radius,
                    is_mobile: ms.is_some(),
                })
                .collect()
        };

    if entities.is_empty() {
        return;
    }

    // Build lookup by entity bits.
    let lookup: BTreeMap<u64, usize> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| (e.bits, i))
        .collect();

    let max_radius = entities
        .iter()
        .map(|e| e.radius)
        .max()
        .unwrap_or(SimFloat::ZERO);

    let grid = world.resource::<SpatialGrid>().clone();
    let push_scale = SimFloat::from_ratio(1, 4);
    let two = SimFloat::TWO;

    // Parallel collision detection.
    let all_displacements: Vec<Vec<(u64, SimVec3)>> = entities
        .par_iter()
        .map(|ce| {
            let mut local_disps: Vec<(u64, SimVec3)> = Vec::new();
            let search_radius = ce.radius + max_radius;
            let center = SimVec2::new(ce.pos_x, ce.pos_z);
            let neighbours = grid.units_in_radius(center, search_radius);

            for neighbour in neighbours {
                let bits_b = neighbour.to_bits();
                if ce.bits >= bits_b {
                    continue;
                }

                let Some(&idx_b) = lookup.get(&bits_b) else {
                    continue;
                };
                let nb = &entities[idx_b];

                let dx = nb.pos_x - ce.pos_x;
                let dz = nb.pos_z - ce.pos_z;
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
                        local_disps.push((ce.bits, SimVec3::new(-push_x, SimFloat::ZERO, -push_z)));
                        local_disps.push((bits_b, SimVec3::new(push_x, SimFloat::ZERO, push_z)));
                    }
                    (true, false) => {
                        local_disps.push((ce.bits, SimVec3::new(-push_x * two, SimFloat::ZERO, -push_z * two)));
                    }
                    (false, true) => {
                        local_disps.push((bits_b, SimVec3::new(push_x * two, SimFloat::ZERO, push_z * two)));
                    }
                    (false, false) => {}
                }
            }
            local_disps
        })
        .collect();

    // Merge deterministically.
    let mut merged: BTreeMap<u64, SimVec3> = BTreeMap::new();
    for disps in &all_displacements {
        for &(bits, disp) in disps {
            merged.entry(bits).and_modify(|d| *d += disp).or_insert(disp);
        }
    }

    // Apply.
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
