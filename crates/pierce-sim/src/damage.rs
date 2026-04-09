//! Damage application, death marking, wreckage spawning, and stun processing.
//!
//! Consumes [`ImpactEventQueue`] from the projectile system and applies
//! damage (or stun) to affected entities, marks dead units, and spawns
//! reclaimable wreckage at the death site.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::combat_data::{ArmorClass, DamageTable};
use crate::components::{Dead, Health, Position, Stunned};
use crate::construction::Reclaimable;
use crate::projectile::ImpactEventQueue;
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Small search radius used for single-target (non-AOE) impacts to find the
/// closest entity at the impact position.
const SINGLE_TARGET_RADIUS: SimFloat = SimFloat::TWO;

/// Default stun duration in simulation frames when hit by a paralyzer weapon.
const PARALYZER_STUN_FRAMES: u32 = 150;

// ---------------------------------------------------------------------------
// DamageEvent (internal)
// ---------------------------------------------------------------------------

/// Internal per-target damage event produced while processing impacts.
struct DamageEvent {
    target: Entity,
    amount: SimFloat,
    is_paralyzer: bool,
}

// ---------------------------------------------------------------------------
// damage_system
// ---------------------------------------------------------------------------

/// Process all pending [`ImpactEvent`]s: apply damage or stun, mark dead
/// entities, and spawn wreckage.
///
/// Should run after [`projectile_movement_system`](crate::projectile::projectile_movement_system)
/// and before [`cleanup_dead`](crate::lifecycle::cleanup_dead).
pub fn damage_system(world: &mut World) {
    // -- 1. Drain impact events ----------------------------------------------
    let impacts: Vec<_> = world
        .resource_mut::<ImpactEventQueue>()
        .events
        .drain(..)
        .collect();

    if impacts.is_empty() {
        return;
    }

    tracing::debug!("damage_system: {} impacts this frame", impacts.len());

    // -- 2. Build per-target damage events -----------------------------------
    let mut damage_events: Vec<DamageEvent> = Vec::new();

    for impact in &impacts {
        let impact_xz = SimVec2::new(impact.position.x, impact.position.z);

        // Find affected entities.
        let targets: Vec<Entity> = if impact.area_of_effect > SimFloat::ZERO {
            world
                .resource::<SpatialGrid>()
                .units_in_radius(impact_xz, impact.area_of_effect)
        } else {
            // Single-target: find closest entity within a small radius.
            let candidates = world
                .resource::<SpatialGrid>()
                .units_in_radius(impact_xz, SINGLE_TARGET_RADIUS);

            // Pick the closest one (by XZ distance).
            let mut best: Option<(Entity, SimFloat)> = None;
            for &entity in &candidates {
                // Only consider entities that have Health (i.e. are damageable).
                if world.get::<Health>(entity).is_none() {
                    continue;
                }
                if let Some(pos) = world.get::<Position>(entity) {
                    let ent_xz = SimVec2::new(pos.pos.x, pos.pos.z);
                    let dist_sq = ent_xz.distance_squared(impact_xz);
                    if best.is_none() || dist_sq < best.unwrap().1 {
                        best = Some((entity, dist_sq));
                    }
                }
            }
            best.map(|(e, _)| e).into_iter().collect()
        };

        // Look up damage table once per impact.
        let table = world.resource::<DamageTable>();

        for &target in &targets {
            // Only damage entities that have Health.
            if world.get::<Health>(target).is_none() {
                continue;
            }

            // Armor class: use the entity's ArmorClass component, default to Light.
            let armor = world
                .get::<ArmorClass>(target)
                .copied()
                .unwrap_or(ArmorClass::Light);

            let multiplier = table.get(impact.damage_type, armor);
            let final_damage = impact.damage * multiplier;

            damage_events.push(DamageEvent {
                target,
                amount: final_damage,
                is_paralyzer: impact.is_paralyzer,
            });
        }
    }

    // -- 3. Apply damage / stun ----------------------------------------------
    for event in &damage_events {
        if event.is_paralyzer {
            // Add or refresh Stunned component.
            if let Some(mut stunned) = world.get_mut::<Stunned>(event.target) {
                // Refresh: reset to full duration.
                stunned.remaining_frames = PARALYZER_STUN_FRAMES;
            } else {
                world.entity_mut(event.target).insert(Stunned {
                    remaining_frames: PARALYZER_STUN_FRAMES,
                });
            }
        } else {
            // Subtract from health.
            if let Some(mut health) = world.get_mut::<Health>(event.target) {
                health.current -= (event.amount.raw() >> 32) as i32;
            }
        }
    }

    // -- 4. Mark dead and spawn wreckage -------------------------------------
    let newly_dead: Vec<(Entity, Position)> = {
        let mut query = world.query::<(Entity, &Health, &Position)>();
        query
            .iter(world)
            .filter(|(entity, health, _)| {
                health.current <= 0 && world.get::<Dead>(*entity).is_none()
            })
            .map(|(entity, _, pos)| (entity, pos.clone()))
            .collect()
    };

    for (entity, position) in &newly_dead {
        // Mark dead.
        world.entity_mut(*entity).insert(Dead);

        // Determine wreckage metal value: 50% of max health as a proxy for
        // unit cost (a real game would look up the unit def's metal cost).
        let base_value = world
            .get::<Health>(*entity)
            .map(|h| SimFloat::from_int(h.max) * SimFloat::HALF)
            .unwrap_or(SimFloat::ZERO);

        // Spawn wreckage entity at the same position.
        world.spawn((
            position.clone(),
            Reclaimable {
                metal_value: base_value,
                reclaim_progress: SimFloat::ZERO,
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// stun_system
// ---------------------------------------------------------------------------

/// Tick down [`Stunned`] timers and remove the component when expired.
///
/// While stunned, the targeting system should skip the entity (checked
/// externally by querying for the [`Stunned`] component).
pub fn stun_system(world: &mut World) {
    let mut expired: Vec<Entity> = Vec::new();

    let mut query = world.query::<(Entity, &mut Stunned)>();
    for (entity, mut stunned) in query.iter_mut(world) {
        if stunned.remaining_frames <= 1 {
            expired.push(entity);
        } else {
            stunned.remaining_frames -= 1;
        }
    }

    for entity in expired {
        world.entity_mut(entity).remove::<Stunned>();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/damage_tests.rs"]
mod tests;
