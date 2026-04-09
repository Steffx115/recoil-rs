//! Energy shield system: regeneration, projectile absorption, and collapse.
//!
//! Shield generators project an energy bubble around the owning entity.
//! Projectiles that enter the shield radius are absorbed, draining shield
//! energy proportional to the projectile's damage. When the shield's energy
//! is fully depleted it collapses (current == 0) and must regenerate before
//! it can absorb again.

use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::components::Position;
use crate::projectile::Projectile;
use crate::{SimFloat, SimVec2};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// Energy shield projected by a shield generator.
///
/// All values use [`SimFloat`] for determinism.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Shield {
    /// Maximum shield energy.
    pub capacity: SimFloat,
    /// Current shield energy (0 = collapsed).
    pub current: SimFloat,
    /// Energy regenerated per simulation tick.
    pub regen_rate: SimFloat,
    /// Radius of the shield bubble in world units.
    pub radius: SimFloat,
}

impl Shield {
    /// Returns `true` when the shield has any energy remaining.
    pub fn is_active(&self) -> bool {
        self.current > SimFloat::ZERO
    }

    /// Absorb damage, returning the amount of damage that passed through
    /// (i.e. was NOT absorbed). If the shield has enough energy the full
    /// damage is absorbed and 0 is returned. If not, the shield collapses
    /// and the excess damage passes through.
    pub fn absorb(&mut self, damage: SimFloat) -> SimFloat {
        if self.current >= damage {
            self.current -= damage;
            SimFloat::ZERO
        } else {
            let excess = damage - self.current;
            self.current = SimFloat::ZERO;
            excess
        }
    }

    /// Regenerate shield energy, clamped to capacity.
    pub fn regenerate(&mut self) {
        if self.current < self.capacity {
            self.current = (self.current + self.regen_rate).min(self.capacity);
        }
    }
}

// ---------------------------------------------------------------------------
// shield_regen_system
// ---------------------------------------------------------------------------

/// Regenerates shield energy for all entities with a [`Shield`] component.
pub fn shield_regen_system(world: &mut World) {
    let entities: Vec<bevy_ecs::entity::Entity> = world
        .query::<(bevy_ecs::entity::Entity, &Shield)>()
        .iter(world)
        .map(|(e, _)| e)
        .collect();

    for entity in entities {
        if let Some(mut shield) = world.get_mut::<Shield>(entity) {
            shield.regenerate();
        }
    }
}

// ---------------------------------------------------------------------------
// shield_absorb_system
// ---------------------------------------------------------------------------

/// Checks in-flight projectiles against shields and absorbs them.
///
/// For each projectile, if its current position is within a shielded
/// entity's shield radius AND the shield is active, the projectile's
/// damage is absorbed. If the shield absorbs the full damage the
/// projectile is destroyed (despawned) and no impact event is created.
/// If the shield cannot fully absorb the damage, the projectile continues
/// with reduced damage.
///
/// Must run BEFORE [`projectile_movement_system`](crate::projectile::projectile_movement_system)
/// so that shields get a chance to intercept before the projectile hits.
pub fn shield_absorb_system(world: &mut World) {
    use bevy_ecs::entity::Entity;

    // Gather all shielded entities.
    struct ShieldInfo {
        entity: Entity,
        pos_xz: SimVec2,
        radius_sq: SimFloat,
    }

    let shields: Vec<ShieldInfo> = {
        let mut query = world.query::<(Entity, &Position, &Shield)>();
        query
            .iter(world)
            .filter(|(_, _, s)| s.is_active())
            .map(|(e, p, s)| ShieldInfo {
                entity: e,
                pos_xz: SimVec2::new(p.pos.x, p.pos.z),
                radius_sq: s.radius * s.radius,
            })
            .collect()
    };

    if shields.is_empty() {
        return;
    }

    // Gather projectile info.
    struct ProjInfo {
        entity: Entity,
        pos_xz: SimVec2,
        damage: SimFloat,
    }

    let projectiles: Vec<ProjInfo> = {
        let mut query = world.query::<(Entity, &Position, &Projectile)>();
        query
            .iter(world)
            .map(|(e, p, proj)| ProjInfo {
                entity: e,
                pos_xz: SimVec2::new(p.pos.x, p.pos.z),
                damage: proj.damage,
            })
            .collect()
    };

    // For each projectile, check against each shield.
    // Use sorted order (by entity bits) for determinism.
    let mut absorbed_projectiles: Vec<Entity> = Vec::new();
    let mut damage_reductions: Vec<(Entity, SimFloat)> = Vec::new();

    for proj in &projectiles {
        let proj_xz = proj.pos_xz;

        // Find the closest active shield that contains this projectile.
        // Sort candidates by distance for determinism.
        let mut candidates: Vec<(SimFloat, usize)> = Vec::new();
        for (i, shield) in shields.iter().enumerate() {
            let dist_sq = proj_xz.distance_squared(shield.pos_xz);
            if dist_sq <= shield.radius_sq {
                candidates.push((dist_sq, i));
            }
        }

        if candidates.is_empty() {
            continue;
        }

        // Pick closest shield (deterministic tie-break by entity bits).
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| shields[a.1].entity.to_bits().cmp(&shields[b.1].entity.to_bits()))
        });

        let shield_idx = candidates[0].1;
        let shield_entity = shields[shield_idx].entity;

        // Try to absorb.
        let excess = {
            let Some(mut shield) = world.get_mut::<Shield>(shield_entity) else {
                continue;
            };
            if !shield.is_active() {
                continue;
            }
            shield.absorb(proj.damage)
        };

        if excess == SimFloat::ZERO {
            // Fully absorbed: mark projectile for despawn.
            absorbed_projectiles.push(proj.entity);
        } else {
            // Partially absorbed: reduce projectile damage.
            damage_reductions.push((proj.entity, excess));
        }
    }

    // Apply damage reductions.
    for (entity, new_damage) in damage_reductions {
        if let Some(mut proj) = world.get_mut::<Projectile>(entity) {
            proj.damage = new_damage;
        }
    }

    // Despawn fully absorbed projectiles.
    for entity in absorbed_projectiles {
        world.despawn(entity);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/shield_tests.rs"]
mod tests;
