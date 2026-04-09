//! Construction, reclaim, and repair mechanics.
//!
//! Builder units contribute their [`Builder::build_power`] each tick toward
//! constructing nanoframes ([`BuildSite`]), reclaiming wreckage
//! ([`Reclaimable`]), or repairing damaged friendlies.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;

use crate::components::{Allegiance, BuildingFootprint, Dead, Health};
use crate::economy::EconomyState;
use crate::footprint::unmark_building_footprint;
use crate::pathfinding::TerrainGrid;
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

/// Remembers a builder's previous build target so it can auto-resume
/// construction when it becomes idle near the unfinished site.
#[derive(Component, Debug, Clone)]
pub struct PreviousBuildTarget {
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

    // Track builders whose BuildTarget should be removed because they are
    // out of range (save as PreviousBuildTarget for auto-resume).
    let mut out_of_range: Vec<(Entity, Entity)> = Vec::new();

    for (builder_entity, build_power, target, team) in builders {
        // Check if target still exists.
        if world.get_entity(target).is_err() {
            continue;
        }

        // Check distance between builder and target.
        let dist_sq_opt = match (
            world.get::<crate::Position>(builder_entity).map(|p| p.pos),
            world.get::<crate::Position>(target).map(|p| p.pos),
        ) {
            (Some(bp), Some(tp)) => {
                let dx = bp.x - tp.x;
                let dz = bp.z - tp.z;
                Some(dx * dx + dz * dz)
            }
            _ => None,
        };

        // Build range: builders must be within 30 world units to apply power.
        let build_range = crate::SimFloat::from_int(30);
        let build_range_sq = build_range * build_range;

        if let Some(dist_sq) = dist_sq_opt {
            // Stop the builder if it's close enough to the target.
            let stop_dist = crate::SimFloat::from_int(15);
            if dist_sq <= stop_dist * stop_dist {
                if let Some(mut ms) = world.get_mut::<crate::MoveState>(builder_entity) {
                    if matches!(*ms, crate::MoveState::MovingTo(_)) {
                        *ms = crate::MoveState::Idle;
                    }
                }
            }

            // If builder is too far from the target, save PreviousBuildTarget
            // and skip construction this tick.  The BuildTarget will be
            // removed after the loop.
            if dist_sq > build_range_sq {
                // Only save for BuildSite targets (not reclaim).
                if world.get::<BuildSite>(target).is_some() {
                    out_of_range.push((builder_entity, target));
                }
                continue;
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

    // Remove BuildTarget from builders that are out of range and save
    // PreviousBuildTarget so they can auto-resume later.
    for (builder_entity, target) in out_of_range {
        world.entity_mut(builder_entity).remove::<BuildTarget>();
        world
            .entity_mut(builder_entity)
            .insert(PreviousBuildTarget { target });
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

        // Repair amount this tick (truncate fractional build power to whole HP).
        let repair_amount = (build_power.raw() >> 32) as i32;
        let new_health = (current + repair_amount).min(max);
        let actual_repair = new_health - current;

        // Resource cost: proportional to repair fraction of max health.
        // Cost per HP = a reasonable fraction; we use 1 metal + 1 energy per HP.
        let actual_repair_sf = SimFloat::from_int(actual_repair);
        let metal_cost = actual_repair_sf;
        let energy_cost = actual_repair_sf;

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
// cancel_build_site
// ---------------------------------------------------------------------------

/// Cancel an in-progress [`BuildSite`], refunding resources proportional to
/// the remaining build progress.
///
/// For example, if a building is 40% complete and costs 100 metal / 200 energy,
/// the refund is 60% of the cost = 60 metal / 120 energy.
///
/// This function:
/// 1. Computes the proportional refund and credits it to the owning team.
/// 2. Unmarks the building footprint on the terrain grid.
/// 3. Marks the entity [`Dead`] for cleanup.
/// 4. Removes [`BuildTarget`] from any builders targeting the cancelled site
///    (and stores a [`PreviousBuildTarget`] so they can remember it).
pub fn cancel_build_site(world: &mut World, site_entity: Entity) {
    // Validate that the entity exists and has a BuildSite.
    if world.get_entity(site_entity).is_err() {
        return;
    }
    let Some(site) = world.get::<BuildSite>(site_entity) else {
        return;
    };

    let progress = site.progress;
    let metal_cost = site.metal_cost;
    let energy_cost = site.energy_cost;

    // Refund = remaining fraction of cost.
    let remaining = SimFloat::ONE - progress;
    let metal_refund = metal_cost * remaining;
    let energy_refund = energy_cost * remaining;

    // Get the team that owns this site.
    let team = world
        .get::<Allegiance>(site_entity)
        .map(|a| a.team)
        .unwrap_or(0);

    // Credit refund to team resources.
    {
        let mut state = world.resource_mut::<EconomyState>();
        if let Some(res) = state.teams.get_mut(&team) {
            res.metal = (res.metal + metal_refund).min(res.metal_storage);
            res.energy = (res.energy + energy_refund).min(res.energy_storage);
        }
    }

    // Unmark the building footprint on the terrain grid.
    if let Some(footprint) = world.get::<BuildingFootprint>(site_entity).cloned() {
        let mut grid = world.resource_mut::<TerrainGrid>();
        unmark_building_footprint(&mut grid, &footprint);
    }

    // Mark the entity dead for cleanup.
    world.entity_mut(site_entity).insert(Dead);

    // Clear BuildTarget from any builders targeting this site.
    let builders_targeting: Vec<Entity> = {
        let mut query = world.query::<(Entity, &BuildTarget)>();
        query
            .iter(world)
            .filter(|(_, bt)| bt.target == site_entity)
            .map(|(e, _)| e)
            .collect()
    };

    for builder_entity in builders_targeting {
        world.entity_mut(builder_entity).remove::<BuildTarget>();
        if let Some(mut ms) = world.get_mut::<crate::MoveState>(builder_entity) {
            *ms = crate::MoveState::Idle;
        }
    }
}

// ---------------------------------------------------------------------------
// auto_resume_construction_system
// ---------------------------------------------------------------------------

/// When a builder's [`BuildTarget`] is removed (e.g., moved away by a move
/// order), save the target as [`PreviousBuildTarget`].  This is called at the
/// start of the construction tick to detect builders that lost their target.
///
/// Then, for idle builders with a [`PreviousBuildTarget`], check if the
/// unfinished [`BuildSite`] still exists and is within resume range.  If so,
/// re-assign the [`BuildTarget`] and start moving toward it.
pub fn auto_resume_construction_system(world: &mut World) {
    // Collect idle builders that have a PreviousBuildTarget but no current BuildTarget.
    let candidates: Vec<(Entity, Entity)> = {
        let mut query = world.query_filtered::<
            (Entity, &PreviousBuildTarget),
            (
                bevy_ecs::query::Without<BuildTarget>,
                bevy_ecs::query::With<Builder>,
            ),
        >();
        query
            .iter(world)
            .map(|(e, pbt)| (e, pbt.target))
            .collect()
    };

    for (builder_entity, prev_target) in candidates {
        // Check that builder is idle.
        let is_idle = world
            .get::<crate::MoveState>(builder_entity)
            .map(|ms| matches!(*ms, crate::MoveState::Idle))
            .unwrap_or(true);

        if !is_idle {
            continue;
        }

        // Check that the previous target still exists and still has a BuildSite.
        if world.get_entity(prev_target).is_err() {
            world
                .entity_mut(builder_entity)
                .remove::<PreviousBuildTarget>();
            continue;
        }
        if world.get::<BuildSite>(prev_target).is_none() {
            // Target is complete or cancelled — forget it.
            world
                .entity_mut(builder_entity)
                .remove::<PreviousBuildTarget>();
            continue;
        }

        // Check proximity: builder must be within 200 world units.
        let in_range = match (
            world.get::<crate::Position>(builder_entity).map(|p| p.pos),
            world.get::<crate::Position>(prev_target).map(|p| p.pos),
        ) {
            (Some(bp), Some(tp)) => {
                let dx = bp.x - tp.x;
                let dz = bp.z - tp.z;
                let dist_sq = dx * dx + dz * dz;
                let resume_dist = SimFloat::from_int(200);
                dist_sq <= resume_dist * resume_dist
            }
            _ => false,
        };

        if in_range {
            // Re-assign build target and start moving.
            world
                .entity_mut(builder_entity)
                .insert(BuildTarget { target: prev_target });
            world
                .entity_mut(builder_entity)
                .remove::<PreviousBuildTarget>();

            if let Some(target_pos) = world.get::<crate::Position>(prev_target).map(|p| p.pos) {
                if let Some(mut ms) = world.get_mut::<crate::MoveState>(builder_entity) {
                    *ms = crate::MoveState::MovingTo(target_pos);
                }
            }
        }
    }
}

/// System to save [`PreviousBuildTarget`] when a builder loses its
/// [`BuildTarget`] while the target still has an unfinished [`BuildSite`].
///
/// Should be called each tick.  It detects builders that have a
/// [`PreviousBuildTarget`] candidate: builders with no [`BuildTarget`]
/// but that previously had one (tracked via the component itself).
///
/// In practice, this is integrated into the construction flow: when a
/// builder's [`BuildTarget`] is removed by a move command (not by
/// construction completion or cancellation), the command system should
/// insert [`PreviousBuildTarget`].  For simplicity, we provide a helper.
pub fn save_previous_build_target(world: &mut World, builder_entity: Entity, target: Entity) {
    // Only save if the target still has a BuildSite (is under construction).
    if world.get::<BuildSite>(target).is_some() {
        world
            .entity_mut(builder_entity)
            .insert(PreviousBuildTarget { target });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/construction_tests.rs"]
mod tests;
