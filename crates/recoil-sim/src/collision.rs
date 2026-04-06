//! Unit-unit collision detection and response.
//!
//! Uses the [`SpatialGrid`] for broad-phase neighbour queries, then does
//! circle-circle overlap tests in the narrow phase.  Overlapping pairs are
//! pushed apart equally along the line between their centres.
//!
//! All math uses deterministic [`SimFloat`] fixed-point arithmetic.

use std::collections::BTreeSet;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;

use crate::components::{CollisionRadius, Position};
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
/// Each pair is processed exactly once thanks to a `BTreeSet` of ordered
/// `(Entity, Entity)` keys.
pub fn collision_system(world: &mut World) {
    // We need the spatial grid to exist as a resource.
    let has_grid = world.get_resource::<SpatialGrid>().is_some();
    if !has_grid {
        return;
    }

    // Collect all collidable entities and their current state.
    let entities: Vec<(Entity, SimVec3, SimFloat)> = world
        .query_filtered::<(Entity, &Position, &CollisionRadius), ()>()
        .iter(world)
        .map(|(e, p, r)| (e, p.pos, r.radius))
        .collect();

    // Find the maximum radius across all entities so we can do a single
    // broad-phase query radius = own_radius + max_other_radius.
    let max_radius = entities
        .iter()
        .map(|&(_, _, r)| r)
        .max()
        .unwrap_or(SimFloat::ZERO);

    // Track which pairs we have already processed.
    let mut processed: BTreeSet<(u64, u64)> = BTreeSet::new();

    // Accumulate displacement vectors so we can apply them all at once
    // (avoids order-dependent position changes within the same tick).
    let mut displacements: Vec<(Entity, SimVec3)> = Vec::new();

    let grid = world.resource::<SpatialGrid>();

    for &(entity_a, pos_a, radius_a) in &entities {
        let search_radius = radius_a + max_radius;
        let center = SimVec2::new(pos_a.x, pos_a.z);
        let neighbours = grid.units_in_radius(center, search_radius);

        for neighbour in neighbours {
            if neighbour == entity_a {
                continue;
            }

            // Canonical pair key (smaller bits first) to avoid double-processing.
            let key = if entity_a.to_bits() < neighbour.to_bits() {
                (entity_a.to_bits(), neighbour.to_bits())
            } else {
                (neighbour.to_bits(), entity_a.to_bits())
            };
            if processed.contains(&key) {
                continue;
            }
            processed.insert(key);

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

            // A moves away from B (negative direction), B moves away from A.
            displacements.push((entity_a, direction * (-half_overlap)));
            displacements.push((neighbour, direction * half_overlap));
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
mod tests {
    use super::*;
    use crate::components::*;
    use crate::spatial::SpatialGrid;

    fn sf(n: i32) -> SimFloat {
        SimFloat::from_int(n)
    }

    fn sv3(x: i32, y: i32, z: i32) -> SimVec3 {
        SimVec3::new(sf(x), sf(y), sf(z))
    }

    /// Build a fresh spatial grid and insert all Position entities.
    fn rebuild_grid(world: &mut World) {
        let mut grid = SpatialGrid::new(sf(10), 20, 20);
        let entities: Vec<(Entity, SimVec3)> = world
            .query::<(Entity, &Position)>()
            .iter(world)
            .map(|(e, p)| (e, p.pos))
            .collect();
        for (e, pos) in entities {
            grid.insert(e, SimVec2::new(pos.x, pos.z));
        }
        world.insert_resource(grid);
    }

    /// Spawn a collidable unit at the given XZ position with the given radius.
    fn spawn_unit(world: &mut World, x: i32, z: i32, radius: i32) -> Entity {
        world
            .spawn((
                Position { pos: sv3(x, 0, z) },
                CollisionRadius { radius: sf(radius) },
            ))
            .id()
    }

    // ---- Two overlapping units get pushed apart ----

    #[test]
    fn overlapping_units_pushed_apart() {
        let mut world = World::new();

        // Two units at x=0 and x=3, each with radius 2 => overlap = 1
        let a = spawn_unit(&mut world, 0, 0, 2);
        let b = spawn_unit(&mut world, 3, 0, 2);

        rebuild_grid(&mut world);
        collision_system(&mut world);

        let pos_a = world.get::<Position>(a).unwrap().pos;
        let pos_b = world.get::<Position>(b).unwrap().pos;

        // They should be pushed apart so distance >= sum of radii (4).
        let dist = pos_a.distance(pos_b);
        assert!(
            dist >= sf(4) - SimFloat::from_ratio(1, 10),
            "units should be pushed apart, dist = {}",
            dist.to_f64()
        );

        // A should have moved left (negative X), B should have moved right.
        assert!(
            pos_a.x < SimFloat::ZERO,
            "A should move left, got x = {}",
            pos_a.x.to_f64()
        );
        assert!(
            pos_b.x > sf(3),
            "B should move right, got x = {}",
            pos_b.x.to_f64()
        );
    }

    // ---- Non-overlapping units stay put ----

    #[test]
    fn non_overlapping_units_stay_put() {
        let mut world = World::new();

        // Two units at x=0 and x=10, each with radius 2 => no overlap
        let a = spawn_unit(&mut world, 0, 0, 2);
        let b = spawn_unit(&mut world, 10, 0, 2);

        rebuild_grid(&mut world);
        collision_system(&mut world);

        let pos_a = world.get::<Position>(a).unwrap().pos;
        let pos_b = world.get::<Position>(b).unwrap().pos;

        assert_eq!(pos_a, sv3(0, 0, 0));
        assert_eq!(pos_b, sv3(10, 0, 0));
    }

    // ---- Multiple overlapping units resolve correctly ----

    #[test]
    fn multiple_overlapping_units_resolve() {
        let mut world = World::new();

        // Three units in a tight cluster — all overlapping.
        let a = spawn_unit(&mut world, 0, 0, 2);
        let b = spawn_unit(&mut world, 2, 0, 2);
        let c = spawn_unit(&mut world, 1, 2, 2);

        rebuild_grid(&mut world);
        collision_system(&mut world);

        let pos_a = world.get::<Position>(a).unwrap().pos;
        let pos_b = world.get::<Position>(b).unwrap().pos;
        let pos_c = world.get::<Position>(c).unwrap().pos;

        // After resolution, all pair-wise distances should be larger than
        // they were initially (they should have been pushed apart).
        let dist_ab = pos_a.distance(pos_b);
        let dist_ac = pos_a.distance(pos_c);
        let dist_bc = pos_b.distance(pos_c);

        // Original distances: AB=2, AC=sqrt(1+4)=~2.24, BC=sqrt(1+4)=~2.24
        // All were < sum_radii=4, so all pairs should have moved apart.
        assert!(
            dist_ab > sf(2),
            "AB should increase, got {}",
            dist_ab.to_f64()
        );
        assert!(
            dist_ac > SimFloat::from_f64(2.24),
            "AC should increase, got {}",
            dist_ac.to_f64()
        );
        assert!(
            dist_bc > SimFloat::from_f64(2.24),
            "BC should increase, got {}",
            dist_bc.to_f64()
        );
    }

    // ---- Determinism: same inputs produce same outputs ----

    #[test]
    fn determinism_same_inputs_same_outputs() {
        fn run_sim() -> Vec<SimVec3> {
            let mut world = World::new();

            let entities: Vec<Entity> = vec![
                spawn_unit(&mut world, 0, 0, 2),
                spawn_unit(&mut world, 2, 0, 2),
                spawn_unit(&mut world, 1, 2, 2),
                spawn_unit(&mut world, 3, 1, 3),
            ];

            // Run multiple ticks to exercise accumulated drift.
            for _ in 0..5 {
                rebuild_grid(&mut world);
                collision_system(&mut world);
            }

            entities
                .iter()
                .map(|&e| world.get::<Position>(e).unwrap().pos)
                .collect()
        }

        let run_a = run_sim();
        let run_b = run_sim();

        assert_eq!(run_a, run_b, "positions must be bit-identical across runs");
    }

    // ---- Coincident centres get separated ----

    #[test]
    fn coincident_centres_separate() {
        let mut world = World::new();

        // Two units at the exact same position.
        let a = spawn_unit(&mut world, 5, 5, 2);
        let b = spawn_unit(&mut world, 5, 5, 2);

        rebuild_grid(&mut world);
        collision_system(&mut world);

        let pos_a = world.get::<Position>(a).unwrap().pos;
        let pos_b = world.get::<Position>(b).unwrap().pos;

        // They should no longer be coincident.
        assert_ne!(pos_a, pos_b, "coincident units must be pushed apart");
    }

    // ---- No grid resource => system is a no-op ----

    #[test]
    fn no_grid_is_noop() {
        let mut world = World::new();
        let a = spawn_unit(&mut world, 0, 0, 2);
        // Do NOT insert a SpatialGrid.
        collision_system(&mut world);
        let pos = world.get::<Position>(a).unwrap().pos;
        assert_eq!(pos, sv3(0, 0, 0));
    }
}
