//! Damage application, death marking, wreckage spawning, and stun processing.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::combat_data::{ArmorClass, DamageTable};
use crate::components::{Dead, Health, Position, Stunned};
use crate::construction::Reclaimable;
use crate::projectile::ImpactEventQueue;
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2};

const SINGLE_TARGET_RADIUS: SimFloat = SimFloat::TWO;
const PARALYZER_STUN_FRAMES: u32 = 150;

/// Process all pending impacts: apply damage/stun, mark dead, spawn wreckage.
pub fn damage_system(world: &mut World) {
    // 1. Drain impacts.
    let impacts: Vec<_> = world
        .resource_mut::<ImpactEventQueue>()
        .events
        .drain(..)
        .collect();

    if impacts.is_empty() {
        return;
    }

    // 2. Pre-collect damageable entities (avoids per-impact ECS random access).
    struct DamageableInfo {
        pos_xz: SimVec2,
        armor: ArmorClass,
        has_health: bool,
    }

    let mut damageables: BTreeMap<u64, DamageableInfo> = BTreeMap::new();
    {
        let mut q = world.query::<(Entity, &Position, Option<&Health>, Option<&ArmorClass>)>();
        for (entity, pos, health, armor) in q.iter(world) {
            if health.is_some() {
                damageables.insert(
                    entity.to_bits(),
                    DamageableInfo {
                        pos_xz: SimVec2::new(pos.pos.x, pos.pos.z),
                        armor: armor.copied().unwrap_or(ArmorClass::Light),
                        has_health: true,
                    },
                );
            }
        }
    }

    let grid = world.resource::<SpatialGrid>();
    let table = world.resource::<DamageTable>().clone();

    // 3. Build per-target damage using pre-collected data + for_each_in_radius.
    let mut damage_map: BTreeMap<u64, (i32, bool)> = BTreeMap::new(); // entity_bits → (total_damage, any_paralyzer)

    for impact in &impacts {
        let impact_xz = SimVec2::new(impact.position.x, impact.position.z);

        if impact.area_of_effect > SimFloat::ZERO {
            // AOE: damage all in radius.
            grid.for_each_in_radius(impact_xz, impact.area_of_effect, |entity, _pos| {
                let bits = entity.to_bits();
                if let Some(info) = damageables.get(&bits) {
                    let mult = table.get(impact.damage_type, info.armor);
                    let dmg = (impact.damage * mult).raw() >> 32;
                    let entry = damage_map.entry(bits).or_insert((0, false));
                    entry.0 += dmg as i32;
                    if impact.is_paralyzer { entry.1 = true; }
                }
            });
        } else {
            // Single target: find closest damageable in small radius.
            let mut best: Option<(u64, SimFloat)> = None;
            grid.for_each_in_radius(impact_xz, SINGLE_TARGET_RADIUS, |entity, _pos| {
                let bits = entity.to_bits();
                if let Some(info) = damageables.get(&bits) {
                    let dist_sq = info.pos_xz.distance_squared(impact_xz);
                    if best.is_none() || dist_sq < best.unwrap().1 {
                        best = Some((bits, dist_sq));
                    }
                }
            });

            if let Some((bits, _)) = best {
                if let Some(info) = damageables.get(&bits) {
                    let mult = table.get(impact.damage_type, info.armor);
                    let dmg = (impact.damage * mult).raw() >> 32;
                    let entry = damage_map.entry(bits).or_insert((0, false));
                    entry.0 += dmg as i32;
                    if impact.is_paralyzer { entry.1 = true; }
                }
            }
        }
    }

    // 4. Apply accumulated damage (one ECS write per target, not per impact).
    for (&bits, &(total_dmg, is_paralyzer)) in &damage_map {
        let entity = Entity::from_bits(bits);

        if is_paralyzer {
            if let Some(mut stunned) = world.get_mut::<Stunned>(entity) {
                stunned.remaining_frames = PARALYZER_STUN_FRAMES;
            } else if world.get_entity(entity).is_ok() {
                world.entity_mut(entity).insert(Stunned {
                    remaining_frames: PARALYZER_STUN_FRAMES,
                });
            }
        } else if let Some(mut health) = world.get_mut::<Health>(entity) {
            health.current -= total_dmg;
        }
    }

    // 5. Mark dead and spawn wreckage.
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
        world.entity_mut(*entity).insert(Dead);

        let base_value = world
            .get::<Health>(*entity)
            .map(|h| SimFloat::from_int(h.max) * SimFloat::HALF)
            .unwrap_or(SimFloat::ZERO);

        world.spawn((
            position.clone(),
            Reclaimable {
                metal_value: base_value,
                reclaim_progress: SimFloat::ZERO,
            },
        ));
    }
}

/// Tick down [`Stunned`] timers and remove the component when expired.
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

#[cfg(test)]
#[path = "tests/damage_tests.rs"]
mod tests;
