//! Unit-unit collision detection and response.
//!
//! Uses pre-collected `SimFrameData` and the `SpatialGrid` for broad-phase.
//! Narrow phase: circle-circle overlap. Overlapping pairs pushed apart.
//! All math uses deterministic `SimFloat`. Rayon parallel.
//!
//! Optimizations:
//! - No BTreeMap: uses flat Vec indexed by insertion order
//! - No per-query Vec allocation: uses `for_each_in_radius` callback
//! - No grid clone: takes reference via Arc-like pattern
//! - No frame data clone: reads directly from resource

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::components::Position;
use crate::frame_data::{CollisionData, SimFrameData};
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2, SimVec3};

/// Run one tick of unit-unit collision detection and response.
pub fn collision_system(world: &mut World) {
    let has_grid = world.get_resource::<SpatialGrid>().is_some();
    if !has_grid {
        return;
    }

    // Collect collision data — use pre-collected or query inline.
    let entities: Vec<CollisionData> =
        if let Some(frame) = world.get_resource::<SimFrameData>() {
            // Clone is unavoidable here: we need owned data for rayon + later mutable world access.
            frame.collision_entities.clone()
        } else {
            use crate::components::{CollisionRadius, MoveState};
            world
                .query_filtered::<(Entity, &Position, &CollisionRadius, Option<&MoveState>), ()>()
                .iter(world)
                .map(|(e, p, r, ms)| CollisionData {
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

    // Build flat lookup: entity bits → index. Sorted for deterministic binary search.
    let mut bits_to_idx: Vec<(u64, usize)> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| (e.bits, i))
        .collect();
    bits_to_idx.sort_unstable_by_key(|&(bits, _)| bits);

    let max_radius = entities
        .iter()
        .map(|e| e.radius)
        .max()
        .unwrap_or(SimFloat::ZERO);

    // Use Arc snapshot from SimFrameData if available (avoids grid clone).
    let grid: std::sync::Arc<SpatialGrid> =
        if let Some(frame) = world.get_resource::<SimFrameData>() {
            if let Some(ref snap) = frame.grid_snapshot {
                snap.clone() // Arc::clone = pointer bump
            } else {
                std::sync::Arc::new(world.resource::<SpatialGrid>().clone())
            }
        } else {
            std::sync::Arc::new(world.resource::<SpatialGrid>().clone())
        };
    let push_scale = SimFloat::from_ratio(1, 4);
    let two = SimFloat::TWO;

    // Parallel collision detection using for_each_in_radius (no Vec alloc per query).
    let all_displacements: Vec<Vec<(usize, SimVec3)>> = entities
        .par_iter()
        .map(|ce| {
            let mut local_disps: Vec<(usize, SimVec3)> = Vec::new();
            let search_radius = ce.radius + max_radius;
            let center = SimVec2::new(ce.pos_x, ce.pos_z);

            grid.for_each_in_radius(center, search_radius, |neighbour, _nb_pos| {
                let bits_b = neighbour.to_bits();
                if ce.bits >= bits_b {
                    return;
                }

                // Binary search in sorted array (O(log n) vs BTreeMap O(log n) but cache-friendlier).
                let idx_b = match bits_to_idx.binary_search_by_key(&bits_b, |&(b, _)| b) {
                    Ok(i) => bits_to_idx[i].1,
                    Err(_) => return,
                };
                let nb = &entities[idx_b];

                let dx = nb.pos_x - ce.pos_x;
                let dz = nb.pos_z - ce.pos_z;
                let dist_sq = dx * dx + dz * dz;
                let sum_radii = ce.radius + nb.radius;
                let sum_radii_sq = sum_radii * sum_radii;

                if dist_sq >= sum_radii_sq {
                    return;
                }

                let (push_x, push_z) = if dist_sq > SimFloat::ZERO {
                    (dx * push_scale, dz * push_scale)
                } else {
                    (ce.radius * push_scale, SimFloat::ZERO)
                };

                // Use index (usize) instead of entity bits for the merge step.
                let idx_a = match bits_to_idx.binary_search_by_key(&ce.bits, |&(b, _)| b) {
                    Ok(i) => bits_to_idx[i].1,
                    Err(_) => return,
                };

                match (ce.is_mobile, nb.is_mobile) {
                    (true, true) => {
                        local_disps.push((idx_a, SimVec3::new(-push_x, SimFloat::ZERO, -push_z)));
                        local_disps.push((idx_b, SimVec3::new(push_x, SimFloat::ZERO, push_z)));
                    }
                    (true, false) => {
                        local_disps.push((idx_a, SimVec3::new(-push_x * two, SimFloat::ZERO, -push_z * two)));
                    }
                    (false, true) => {
                        local_disps.push((idx_b, SimVec3::new(push_x * two, SimFloat::ZERO, push_z * two)));
                    }
                    (false, false) => {}
                }
            });
            local_disps
        })
        .collect();

    // Merge into flat array indexed by entity index (no BTreeMap).
    let mut merged = vec![SimVec3::ZERO; entities.len()];
    for disps in &all_displacements {
        for &(idx, disp) in disps {
            merged[idx] += disp;
        }
    }

    // Apply displacements in entity order (deterministic — same order as entities vec).
    for (i, disp) in merged.iter().enumerate() {
        if *disp == SimVec3::ZERO {
            continue;
        }
        let entity = entities[i].entity;
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
