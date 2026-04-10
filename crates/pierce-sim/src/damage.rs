//! Damage application, death marking, wreckage spawning, and stun processing.

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;

use crate::combat_data::{ArmorClass, DamageTable};
use crate::components::{Dead, Health, Position, Stunned};
use crate::construction::Reclaimable;
use crate::projectile::ImpactEventQueue;
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2};

const SINGLE_TARGET_RADIUS: SimFloat = SimFloat::TWO;
const PARALYZER_STUN_FRAMES: u32 = 150;

// ---------------------------------------------------------------------------
// DamageableCache — pre-collected per frame, shared across systems
// ---------------------------------------------------------------------------

/// Flat-array cache of damageable entity data, indexed by `Entity::index()`.
/// O(1) lookup instead of binary search. Populated by
/// `projectile_movement_system` (or lazily by `damage_system`).
#[derive(Resource, Default)]
pub struct DamageableCache {
    /// Indexed by `Entity::index()`. `None` = not damageable.
    entries: Vec<Option<DamageableEntry>>,
    /// Track whether the cache has been populated this frame.
    populated: bool,
}

#[derive(Clone, Copy)]
pub struct DamageableEntry {
    pub pos_xz: SimVec2,
    pub armor: ArmorClass,
}

impl DamageableCache {
    pub fn clear(&mut self) {
        // Zero out the flags without deallocating.
        for slot in &mut self.entries {
            *slot = None;
        }
        self.populated = false;
    }

    /// O(1) lookup by entity index.
    #[inline]
    pub fn get(&self, entity: Entity) -> Option<&DamageableEntry> {
        self.entries
            .get(entity.index() as usize)
            .and_then(|slot| slot.as_ref())
    }

    #[inline]
    pub fn is_populated(&self) -> bool {
        self.populated
    }
}

/// Populate the [`DamageableCache`] from the current world state.
/// Called by `projectile_movement_system` to pre-warm the cache, or
/// lazily by `damage_system` if the cache is still empty.
pub fn populate_damageable_cache(world: &mut World) {
    let mut cache = world.resource_mut::<DamageableCache>();
    cache.clear();

    // Take the vec out to avoid borrow conflict with world queries.
    let mut entries = std::mem::take(&mut cache.entries);
    for slot in &mut entries {
        *slot = None;
    }

    let mut q = world.query::<(Entity, &Position, Option<&Health>, Option<&ArmorClass>)>();
    for (entity, pos, health, armor) in q.iter(world) {
        if health.is_some() {
            let idx = entity.index() as usize;
            if idx >= entries.len() {
                entries.resize(idx + 1, None);
            }
            entries[idx] = Some(DamageableEntry {
                pos_xz: SimVec2::new(pos.pos.x, pos.pos.z),
                armor: armor.copied().unwrap_or(ArmorClass::Light),
            });
        }
    }

    // Put it back.
    let mut cache = world.resource_mut::<DamageableCache>();
    cache.entries = entries;
    cache.populated = true;
}

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

    // 2. Ensure damageable cache is populated (lazy fallback if projectile
    //    system didn't pre-warm it, e.g. in tests).
    if !world.resource::<DamageableCache>().is_populated() {
        populate_damageable_cache(world);
    }

    let grid = world.resource::<SpatialGrid>();
    let table = world.resource::<DamageTable>().clone();
    let cache = world.resource::<DamageableCache>();

    // 3. Accumulate damage per target.
    //    Flat array indexed by entity index for O(1) accumulation.
    //    Track which entities were touched to avoid scanning the whole array.
    let mut damage_accum: Vec<(i32, bool)> = Vec::new(); // indexed by entity index
    let mut touched: Vec<Entity> = Vec::new(); // entities that received damage

    /// Accumulate damage for an entity into the flat array.
    #[inline]
    fn accum(
        damage_accum: &mut Vec<(i32, bool)>,
        touched: &mut Vec<Entity>,
        entity: Entity,
        dmg: i32,
        is_paralyzer: bool,
    ) {
        let idx = entity.index() as usize;
        if idx >= damage_accum.len() {
            damage_accum.resize(idx + 1, (0, false));
        }
        let entry = &mut damage_accum[idx];
        if entry.0 == 0 && !entry.1 {
            touched.push(entity);
        }
        entry.0 += dmg;
        entry.1 |= is_paralyzer;
    }

    for impact in &impacts {
        if impact.area_of_effect > SimFloat::ZERO {
            // AOE: damage all in radius.
            let impact_xz = SimVec2::new(impact.position.x, impact.position.z);
            grid.for_each_in_radius(impact_xz, impact.area_of_effect, |entity, _pos| {
                if let Some(info) = cache.get(entity) {
                    let mult = table.get(impact.damage_type, info.armor);
                    let dmg = (impact.damage * mult).raw() >> 32;
                    accum(&mut damage_accum, &mut touched, entity, dmg as i32, impact.is_paralyzer);
                }
            });
        } else if let Some(target) = impact.target_entity {
            // Direct hit with known target — skip spatial query entirely.
            if let Some(info) = cache.get(target) {
                let mult = table.get(impact.damage_type, info.armor);
                let dmg = (impact.damage * mult).raw() >> 32;
                accum(&mut damage_accum, &mut touched, target, dmg as i32, impact.is_paralyzer);
            }
        } else {
            // Single target without known entity: find closest damageable.
            let impact_xz = SimVec2::new(impact.position.x, impact.position.z);
            let mut best: Option<(Entity, SimFloat)> = None;
            grid.for_each_in_radius(impact_xz, SINGLE_TARGET_RADIUS, |entity, _pos| {
                if let Some(info) = cache.get(entity) {
                    let dist_sq = info.pos_xz.distance_squared(impact_xz);
                    if best.is_none() || dist_sq < best.unwrap().1 {
                        best = Some((entity, dist_sq));
                    }
                }
            });

            if let Some((entity, _)) = best {
                if let Some(info) = cache.get(entity) {
                    let mult = table.get(impact.damage_type, info.armor);
                    let dmg = (impact.damage * mult).raw() >> 32;
                    accum(&mut damage_accum, &mut touched, entity, dmg as i32, impact.is_paralyzer);
                }
            }
        }
    }

    // 4. Apply accumulated damage. Sort by entity index for deterministic order.
    touched.sort_unstable_by_key(|e| e.index());

    for &entity in &touched {
        let (total_dmg, is_paralyzer) = damage_accum[entity.index() as usize];

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

    // 6. Mark dead and spawn wreckage.
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
