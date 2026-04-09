//! Per-frame pre-allocated data buffers to avoid repeated allocations.
//!
//! At 40K+ units, collecting entity data into fresh Vecs every frame
//! causes significant allocation overhead. This resource persists across
//! frames and reuses its buffers via `clear()` (which preserves capacity).

use std::sync::Arc;

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;

use crate::spatial::SpatialGrid;
use crate::targeting::WeaponRegistry;
use crate::{SimFloat, SimVec2, SimVec3};

/// Cached entity data for spatial grid, collision, and movement.
/// Collected once per frame, shared across systems.
#[derive(Resource)]
pub struct SimFrameData {
    /// (Entity, position_xz) for spatial grid rebuild.
    pub spatial_entries: Vec<(Entity, SimVec2)>,

    /// Collision entity data.
    pub collision_entities: Vec<CollisionData>,

    /// Displacement accumulation for collision.
    pub displacements: Vec<(u64, SimVec3)>,

    /// Snapshot of the spatial grid after rebuild. Arc so rayon threads
    /// can share it without cloning the entire grid.
    pub grid_snapshot: Option<Arc<SpatialGrid>>,

    /// Snapshot of weapon registry. Arc avoids 4× full clone per frame.
    pub registry_snapshot: Option<Arc<WeaponRegistry>>,
}

/// Pre-collected collision data.
#[derive(Clone, Copy)]
pub struct CollisionData {
    pub entity: Entity,
    pub bits: u64,
    pub pos_x: SimFloat,
    pub pos_z: SimFloat,
    pub radius: SimFloat,
    pub is_mobile: bool,
}

impl Default for SimFrameData {
    fn default() -> Self {
        Self {
            spatial_entries: Vec::with_capacity(4096),
            collision_entities: Vec::with_capacity(4096),
            displacements: Vec::with_capacity(4096),
            grid_snapshot: None,
            registry_snapshot: None,
        }
    }
}

impl SimFrameData {
    /// Clear all buffers for the next frame (preserves allocation capacity).
    pub fn clear(&mut self) {
        self.spatial_entries.clear();
        self.collision_entities.clear();
        self.displacements.clear();
        self.grid_snapshot = None;
        // Registry snapshot persists — it doesn't change between frames.
    }
}
