//! Construction, reclaim, and repair mechanics.
//!
//! Builder units contribute their [`Builder::build_power`] each tick toward
//! constructing nanoframes ([`BuildSite`]), reclaiming wreckage
//! ([`Reclaimable`]), or repairing damaged friendlies.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;

use crate::components::{Allegiance, Dead, Health};
use crate::economy::EconomyState;
use crate::SimFloat;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// How fast this unit builds, reclaims, or repairs per tick.
#[derive(Component, Debug, Clone)]
pub struct Builder {
    pub build_power: SimFloat,
}

/// Placed on a nanoframe entity that is under construction.
#[derive(Component, Debug, Clone)]
pub struct BuildSite {
    pub metal_cost: SimFloat,
    pub energy_cost: SimFloat,
    pub total_build_time: SimFloat,
    /// Progress in 0..1 range.
    pub progress: SimFloat,
}

/// Placed on wreckage / features that can be reclaimed for metal.
#[derive(Component, Debug, Clone)]
pub struct Reclaimable {
    pub metal_value: SimFloat,
    /// Progress in 0..1 range — when it reaches 1 the entity is despawned.
    pub reclaim_progress: SimFloat,
}

/// Placed on a builder to indicate what it is currently building or reclaiming.
#[derive(Component, Debug, Clone)]
pub struct BuildTarget {
    pub target: Entity,
}

// ---------------------------------------------------------------------------
// ConstructionTarget trait
// ---------------------------------------------------------------------------

/// Result of applying build power to a construction target.
pub struct BuildResult {
    /// Whether the target reached completion this tick.
    pub completed: bool,
    /// Metal consumed this tick.
    pub metal_consumed: SimFloat,
    /// Energy consumed this tick.
    pub energy_consumed: SimFloat,
}

/// Unified interface for entities that accept builder work (nanoframes and
/// reclaimable wreckage). Allows `construction_system` to dispatch without
/// branching on component type.
pub trait ConstructionTarget {
    /// Apply `build_power` worth of work, scaled by the economy `stall` ratio
    /// (1.0 = no stall). Returns completion status and resource cost.
    fn accept_build_power(&mut self, power: SimFloat, stall: SimFloat) -> BuildResult;

    /// The (metal, energy) cost of this target. For reclaimables the cost is
    /// zero (reclaiming produces resources, not consumes them).
    fn resource_cost(&self) -> (SimFloat, SimFloat);
}

impl ConstructionTarget for BuildSite {
    fn accept_build_power(&mut self, power: SimFloat, stall: SimFloat) -> BuildResult {
        let progress_delta = power / self.total_build_time;
        let metal_needed = self.metal_cost * progress_delta;
        let energy_needed = self.energy_cost * progress_delta;

        let effective_delta = progress_delta * stall;
        let effective_metal = metal_needed * stall;
        let effective_energy = energy_needed * stall;

        self.progress += effective_delta;
        let completed = if self.progress >= SimFloat::ONE {
            self.progress = SimFloat::ONE;
            true
        } else {
            false
        };

        BuildResult {
            completed,
            metal_consumed: effective_metal,
            energy_consumed: effective_energy,
        }
    }

    fn resource_cost(&self) -> (SimFloat, SimFloat) {
        (self.metal_cost, self.energy_cost)
    }
}

impl ConstructionTarget for Reclaimable {
    fn accept_build_power(&mut self, power: SimFloat, _stall: SimFloat) -> BuildResult {
        let progress_delta = if self.metal_value > SimFloat::ZERO {
            power / self.metal_value
        } else {
            SimFloat::ONE
        };

        self.reclaim_progress += progress_delta;
        let completed = if self.reclaim_progress >= SimFloat::ONE {
            self.reclaim_progress = SimFloat::ONE;
            true
        } else {
            false
        };

        // Reclaim does not consume resources.
        BuildResult {
            completed,
            metal_consumed: SimFloat::ZERO,
            energy_consumed: SimFloat::ZERO,
        }
    }

    fn resource_cost(&self) -> (SimFloat, SimFloat) {
        (SimFloat::ZERO, SimFloat::ZERO)
    }
}

// ---------------------------------------------------------------------------
// construction_system
// ---------------------------------------------------------------------------

/// Run one tick of construction and reclaim processing.
///
/// For every entity with [`Builder`], [`BuildTarget`], and [`Allegiance`]:
/// - If the target has a [`BuildSite`]: contribute build power, consume
///   resources, and complete when progress reaches 1.
/// - If the target has a [`Reclaimable`]: contribute build power toward
///   reclaim, and when complete add metal to team resources and mark the
///   target [`Dead`].
pub fn construction_system(world: &mut World) {
    // Collect builder data up front to avoid borrow conflicts.
    let builders: Vec<(Entity, SimFloat, Entity, u8)> = {
        let mut query = world.query::<(Entity, &Builder, &BuildTarget, &Allegiance)>();
        query
            .iter(world)
            .map(|(e, b, bt, a)| (e, b.build_power, bt.target, a.team))
            .collect()
    };

    for (builder_entity, build_power, target, team) in builders {
        // Check if target still exists.
        if world.get_entity(target).is_err() {
            continue;
        }

        // Stop the builder if it's close enough to the target.
        if let (Some(builder_pos), Some(target_pos)) = (
            world.get::<crate::Position>(builder_entity).map(|p| p.pos),
            world.get::<crate::Position>(target).map(|p| p.pos),
        ) {
            let dx = builder_pos.x - target_pos.x;
            let dz = builder_pos.z - target_pos.z;
            let dist_sq = dx * dx + dz * dz;
            // Stop within ~15 world units of the target.
            let stop_dist = crate::SimFloat::from_int(15);
            if dist_sq <= stop_dist * stop_dist {
                if let Some(mut ms) = world.get_mut::<crate::MoveState>(builder_entity) {
                    if matches!(*ms, crate::MoveState::MovingTo(_)) {
                        *ms = crate::MoveState::Idle;
                    }
                }
            }
        }

        // --- BuildSite target (construction) ---
        if world.get::<BuildSite>(target).is_some() {
            // Fetch economy stall for this team.
            let stall = {
                let state = world.resource::<EconomyState>();
                if let Some(res) = state.teams.get(&team) {
                    res.stall_ratio_metal.min(res.stall_ratio_energy)
                } else {
                    SimFloat::ONE
                }
            };

            let result = {
                let mut site = world.get_mut::<BuildSite>(target).unwrap();
                site.accept_build_power(build_power, stall)
            };

            // Deduct resources from team.
            {
                let mut state = world.resource_mut::<EconomyState>();
                if let Some(res) = state.teams.get_mut(&team) {
                    res.metal = (res.metal - result.metal_consumed).max(SimFloat::ZERO);
                    res.energy = (res.energy - result.energy_consumed).max(SimFloat::ZERO);
                }
            }

            if result.completed {
                // Remove BuildSite, set Health to max.
                world.entity_mut(target).remove::<BuildSite>();
                if let Some(mut health) = world.get_mut::<Health>(target) {
                    health.current = health.max;
                }

                // Clear builder's BuildTarget and stop movement so it doesn't
                // keep walking toward the completed building.
                world.entity_mut(builder_entity).remove::<BuildTarget>();
                if let Some(mut ms) = world.get_mut::<crate::MoveState>(builder_entity) {
                    *ms = crate::MoveState::Idle;
                }
            }

            continue;
        }

        // --- Reclaimable target (reclaim) ---
        if world.get::<Reclaimable>(target).is_some() {
            let metal_value = world.get::<Reclaimable>(target).unwrap().metal_value;

            let result = {
                let mut reclaimable = world.get_mut::<Reclaimable>(target).unwrap();
                reclaimable.accept_build_power(build_power, SimFloat::ONE)
            };

            if result.completed {
                // Add metal to team resources.
                {
                    let mut state = world.resource_mut::<EconomyState>();
                    if let Some(res) = state.teams.get_mut(&team) {
                        res.metal = (res.metal + metal_value).min(res.metal_storage);
                    }
                }

                // Mark target dead for cleanup.
                world.entity_mut(target).insert(Dead);

                // Clear builder's BuildTarget and stop movement.
                world.entity_mut(builder_entity).remove::<BuildTarget>();
                if let Some(mut ms) = world.get_mut::<crate::MoveState>(builder_entity) {
                    *ms = crate::MoveState::Idle;
                }
            }

            continue;
        }
    }
}

// ---------------------------------------------------------------------------
// repair_system
// ---------------------------------------------------------------------------

/// Run one tick of repair processing.
///
/// For every builder targeting an entity with [`Health`] (but no
/// [`BuildSite`]): restore health at `build_power` rate, consuming
/// resources proportionally.
pub fn repair_system(world: &mut World) {
    let builders: Vec<(SimFloat, Entity, u8)> = {
        let mut query = world.query::<(&Builder, &BuildTarget, &Allegiance)>();
        query
            .iter(world)
            .map(|(b, bt, a)| (b.build_power, bt.target, a.team))
            .collect()
    };

    for (build_power, target, team) in builders {
        // Target must exist, have Health, and NOT have a BuildSite.
        if world.get_entity(target).is_err() {
            continue;
        }
        if world.get::<BuildSite>(target).is_some() {
            continue;
        }
        let Some(health) = world.get::<Health>(target) else {
            continue;
        };

        let current = health.current;
        let max = health.max;

        // Already at full health — nothing to do.
        if current >= max {
            continue;
        }

        // Repair amount this tick.
        let repair_amount = build_power;
        let new_health = (current + repair_amount).min(max);
        let actual_repair = new_health - current;

        // Resource cost: proportional to repair fraction of max health.
        // Cost per HP = a reasonable fraction; we use 1 metal + 1 energy per HP.
        let metal_cost = actual_repair;
        let energy_cost = actual_repair;

        // Deduct resources.
        {
            let mut state = world.resource_mut::<EconomyState>();
            if let Some(res) = state.teams.get_mut(&team) {
                res.metal = (res.metal - metal_cost).max(SimFloat::ZERO);
                res.energy = (res.energy - energy_cost).max(SimFloat::ZERO);
            }
        }

        // Apply repair.
        if let Some(mut health) = world.get_mut::<Health>(target) {
            health.current = new_health;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/construction_tests.rs"]
mod tests;
