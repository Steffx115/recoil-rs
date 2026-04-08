//! Unit selection state and screen-to-world ray casting.
//!
//! [`SelectionState`] tracks which entities are currently selected and
//! supports control groups (0–9), shift-click toggle, and drag-box
//! selection.  The [`screen_to_ground_raw`] helper unprojects screen
//! coordinates through an inverse view-projection matrix and intersects
//! the resulting ray with the ground plane (y = 0).

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::World;

use crate::components::UnitType;

// ---------------------------------------------------------------------------
// SelectionState
// ---------------------------------------------------------------------------

/// Tracks which entities the local player has selected plus 10 control
/// groups (keys 0–9).
#[derive(Debug, Default)]
pub struct SelectionState {
    /// Currently selected entities, in selection order.
    pub selected: Vec<Entity>,
    /// Control groups mapped by slot 0–9.
    pub control_groups: BTreeMap<u8, Vec<Entity>>,
}

impl SelectionState {
    /// Clear the current selection and select a single entity.
    pub fn select_single(&mut self, entity: Entity) {
        self.selected.clear();
        self.selected.push(entity);
    }

    /// Toggle an entity in the current selection (shift-click).
    ///
    /// If the entity is already selected it is removed; otherwise it is
    /// appended.
    pub fn toggle(&mut self, entity: Entity) {
        if let Some(idx) = self.selected.iter().position(|&e| e == entity) {
            self.selected.remove(idx);
        } else {
            self.selected.push(entity);
        }
    }

    /// Replace the current selection with the entities inside a drag box.
    pub fn select_box(&mut self, entities: Vec<Entity>) {
        self.selected = entities;
    }

    /// Clear the selection entirely.
    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// Save the current selection into a control-group slot (0–9).
    pub fn save_control_group(&mut self, slot: u8) {
        self.control_groups.insert(slot, self.selected.clone());
    }

    /// Recall a previously saved control group, replacing the current
    /// selection.
    pub fn recall_control_group(&mut self, slot: u8) {
        if let Some(group) = self.control_groups.get(&slot) {
            self.selected = group.clone();
        }
    }

    /// Select every entity in the world that shares the given
    /// [`UnitType`] id.  This implements double-click "select all of
    /// type" behaviour.
    pub fn select_all_of_type(&mut self, world: &mut World, unit_type_id: u32) {
        let mut entities = Vec::new();
        let mut query = world.query::<(Entity, &UnitType)>();
        for (entity, ut) in query.iter(world) {
            if ut.id == unit_type_id {
                entities.push(entity);
            }
        }
        // Sort for determinism (Entity ordering is stable within a World).
        entities.sort();
        self.selected = entities;
    }
}

// ---------------------------------------------------------------------------
// Screen-to-ground ray casting
// ---------------------------------------------------------------------------

/// Unproject screen coordinates through the given inverse view-projection
/// matrix and intersect the resulting ray with the ground plane (y = 0).
///
/// `inv_view_proj` must be a column-major 4x4 matrix (the *inverse* of the
/// combined view-projection matrix).
///
/// Returns `Some((world_x, world_z))` if the ray hits the ground plane, or
/// `None` if the ray is parallel to the plane.
pub fn screen_to_ground_raw(
    screen_x: f32,
    screen_y: f32,
    screen_w: f32,
    screen_h: f32,
    inv_view_proj: &[[f32; 4]; 4],
) -> Option<(f32, f32)> {
    // 1. Convert screen coords to NDC [-1, 1].
    let ndc_x = (2.0 * screen_x / screen_w) - 1.0;
    let ndc_y = 1.0 - (2.0 * screen_y / screen_h); // flip Y

    // 2. Unproject near and far points to get a world-space ray.
    let near_pt = mat4_mul_point(inv_view_proj, [ndc_x, ndc_y, 0.0, 1.0]);
    let far_pt = mat4_mul_point(inv_view_proj, [ndc_x, ndc_y, 1.0, 1.0]);

    let near = perspective_divide(near_pt)?;
    let far = perspective_divide(far_pt)?;

    // 3. Direction from near to far.
    let dir = [far[0] - near[0], far[1] - near[1], far[2] - near[2]];

    // 4. Intersect with y = 0 plane.
    if dir[1].abs() < 1e-9 {
        return None; // Ray parallel to ground
    }

    let t = -near[1] / dir[1];
    // We accept any t — the camera may be above or below the plane, and we
    // want the intersection regardless of direction.
    let world_x = near[0] + t * dir[0];
    let world_z = near[2] + t * dir[2];
    Some((world_x, world_z))
}

/// Multiply a column-major 4x4 matrix by a 4-component vector.
fn mat4_mul_point(m: &[[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2] + m[3][0] * v[3],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2] + m[3][1] * v[3],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2] + m[3][2] * v[3],
        m[0][3] * v[0] + m[1][3] * v[1] + m[2][3] * v[2] + m[3][3] * v[3],
    ]
}

/// Perform perspective divide, returning `None` if w is near zero.
fn perspective_divide(p: [f32; 4]) -> Option<[f32; 3]> {
    if p[3].abs() < 1e-10 {
        return None;
    }
    Some([p[0] / p[3], p[1] / p[3], p[2] / p[3]])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
