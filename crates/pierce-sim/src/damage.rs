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

/// Cached damageable entity data, sorted by entity bits for binary search.
/// Populated by `projectile_movement_system` (or lazily by `damage_system`
/// if the cache is empty) so the data is ready before damage processing.
#[derive(Resource, Default)]
pub struct DamageableCache {
    /// Sorted by `bits` for binary-search lookup.
    pub entries: Vec<DamageableEntry>,
}

#[derive(Clone, Copy)]
pub struct DamageableEntry {
    pub bits: u64,
    pub pos_xz: SimVec2,
    pub armor: ArmorClass,
}

impl DamageableCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Look up by entity bits (binary search — entries must be sorted).
    #[inline]
    pub fn get(&self, bits: u64) -> Option<&DamageableEntry> {
        self.entries
            .binary_search_by_key(&bits, |e| e.bits)
            .ok()
            .map(|i| &self.entries[i])
    }
}

/// Populate the [`DamageableCache`] from the current world state.
/// Called by `projectile_movement_system` to pre-warm the cache, or
/// lazily by `damage_system` if the cache is still empty.
pub fn populate_damageable_cache(world: &mut World) {
    let mut cache = world.resource_mut::<DamageableCache>();
    cache.clear();

    // Collect into local vec, then sort.
    let mut entries = std::mem::take(&mut cache.entries);
    entries.clear();

    let mut q = world.query::<(Entity, &Position, Option<&Health>, Option<&ArmorClass>)>();
    for (entity, pos, health, armor) in q.iter(world) {
        if health.is_some() {
            entries.push(DamageableEntry {
                bits: entity.to_bits(),
                pos_xz: SimVec2::new(pos.pos.x, pos.pos.z),
                armor: armor.copied().unwrap_or(ArmorClass::Light),
            });
        }
    }

    entries.sort_unstable_by_key(|e| e.bits);

    // Put it back.
    let mut cache = world.resource_mut::<DamageableCache>();
    cache.entries = entries;
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
    if world.resource::<DamageableCache>().entries.is_empty() {
        populate_damageable_cache(world);
    }

    let grid = world.resource::<SpatialGrid>();
    let table = world.resource::<DamageTable>().clone();
    let cache = world.resource::<DamageableCache>();

    // 3. Build per-target damage using cache + for_each_in_radius.
    //    Vec of (entity_bits, total_damage, any_paralyzer), accumulated then sorted.
    let mut damage_list: Vec<(u64, i32, bool)> = Vec::new();

    for impact in &impacts {
        let impact_xz = SimVec2::new(impact.position.x, impact.position.z);

        if impact.area_of_effect > SimFloat::ZERO {
            // AOE: damage all in radius.
            grid.for_each_in_radius(impact_xz, impact.area_of_effect, |entity, _pos| {
                let bits = entity.to_bits();
                if let Some(info) = cache.get(bits) {
                    let mult = table.get(impact.damage_type, info.armor);
                    let dmg = (impact.damage * mult).raw() >> 32;
                    damage_list.push((bits, dmg as i32, impact.is_paralyzer));
                }
            });
        } else {
            // Single target: find closest damageable in small radius.
            let mut best: Option<(u64, SimFloat)> = None;
            grid.for_each_in_radius(impact_xz, SINGLE_TARGET_RADIUS, |entity, _pos| {
                let bits = entity.to_bits();
                if let Some(info) = cache.get(bits) {
                    let dist_sq = info.pos_xz.distance_squared(impact_xz);
                    if best.is_none() || dist_sq < best.unwrap().1 {
                        best = Some((bits, dist_sq));
                    }
                }
            });

            if let Some((bits, _)) = best {
                if let Some(info) = cache.get(bits) {
                    let mult = table.get(impact.damage_type, info.armor);
                    let dmg = (impact.damage * mult).raw() >> 32;
                    damage_list.push((bits, dmg as i32, impact.is_paralyzer));
                }
            }
        }
    }

    // 4. Sort by entity bits and merge duplicates into accumulated totals.
    damage_list.sort_unstable_by_key(|&(bits, _, _)| bits);

    let mut merged: Vec<(u64, i32, bool)> = Vec::new();
    for (bits, dmg, para) in damage_list {
        if let Some(last) = merged.last_mut() {
            if last.0 == bits {
                last.1 += dmg;
                last.2 |= para;
                continue;
            }
        }
        merged.push((bits, dmg, para));
    }

    // 5. Apply accumulated damage (one ECS write per target, not per impact).
    for &(bits, total_dmg, is_paralyzer) in &merged {
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
