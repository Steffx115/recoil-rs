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
mod tests {
    use super::*;
    use crate::components::UnitType;
    use bevy_ecs::prelude::World;

    // ---- SelectionState basics ----

    #[test]
    fn select_single_clears_and_sets() {
        let mut world = World::new();
        let e1 = world.spawn_empty().id();
        let e2 = world.spawn_empty().id();

        let mut sel = SelectionState::default();
        sel.select_single(e1);
        assert_eq!(sel.selected, vec![e1]);

        sel.select_single(e2);
        assert_eq!(sel.selected, vec![e2]);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut world = World::new();
        let e1 = world.spawn_empty().id();
        let e2 = world.spawn_empty().id();

        let mut sel = SelectionState::default();
        sel.toggle(e1);
        sel.toggle(e2);
        assert_eq!(sel.selected.len(), 2);

        // Toggle e1 off
        sel.toggle(e1);
        assert_eq!(sel.selected, vec![e2]);

        // Toggle e2 off
        sel.toggle(e2);
        assert!(sel.selected.is_empty());
    }

    #[test]
    fn select_box_replaces_selection() {
        let mut world = World::new();
        let e1 = world.spawn_empty().id();
        let e2 = world.spawn_empty().id();
        let e3 = world.spawn_empty().id();

        let mut sel = SelectionState::default();
        sel.select_single(e1);
        sel.select_box(vec![e2, e3]);
        assert_eq!(sel.selected, vec![e2, e3]);
    }

    #[test]
    fn clear_empties_selection() {
        let mut world = World::new();
        let e1 = world.spawn_empty().id();

        let mut sel = SelectionState::default();
        sel.select_single(e1);
        sel.clear();
        assert!(sel.selected.is_empty());
    }

    // ---- Control groups ----

    #[test]
    fn save_and_recall_control_group() {
        let mut world = World::new();
        let e1 = world.spawn_empty().id();
        let e2 = world.spawn_empty().id();

        let mut sel = SelectionState::default();
        sel.select_box(vec![e1, e2]);
        sel.save_control_group(1);

        // Change selection
        sel.clear();
        assert!(sel.selected.is_empty());

        // Recall
        sel.recall_control_group(1);
        assert_eq!(sel.selected, vec![e1, e2]);
    }

    #[test]
    fn recall_nonexistent_group_does_nothing() {
        let mut sel = SelectionState::default();
        sel.recall_control_group(5);
        assert!(sel.selected.is_empty());
    }

    #[test]
    fn control_groups_use_slots_0_to_9() {
        let mut world = World::new();
        let e = world.spawn_empty().id();

        let mut sel = SelectionState::default();
        for slot in 0..10u8 {
            sel.select_single(e);
            sel.save_control_group(slot);
        }
        assert_eq!(sel.control_groups.len(), 10);
        for slot in 0..10u8 {
            sel.recall_control_group(slot);
            assert_eq!(sel.selected, vec![e]);
        }
    }

    // ---- select_all_of_type ----

    #[test]
    fn select_all_of_type_filters_correctly() {
        let mut world = World::new();
        let tank1 = world.spawn(UnitType { id: 1 }).id();
        let _scout = world.spawn(UnitType { id: 2 }).id();
        let tank2 = world.spawn(UnitType { id: 1 }).id();

        let mut sel = SelectionState::default();
        sel.select_all_of_type(&mut world, 1);

        assert_eq!(sel.selected.len(), 2);
        assert!(sel.selected.contains(&tank1));
        assert!(sel.selected.contains(&tank2));
    }

    #[test]
    fn select_all_of_type_empty_when_none_match() {
        let mut world = World::new();
        let _scout = world.spawn(UnitType { id: 2 }).id();

        let mut sel = SelectionState::default();
        sel.select_all_of_type(&mut world, 99);
        assert!(sel.selected.is_empty());
    }

    // ---- screen_to_ground_raw ----

    #[test]
    fn screen_center_hits_ground() {
        // Build a simple camera: looking down -Z from (0, 10, 0).
        let inv_vp = build_test_inv_vp();
        let result = screen_to_ground_raw(400.0, 300.0, 800.0, 600.0, &inv_vp);
        assert!(result.is_some(), "center of screen should hit ground");
    }

    #[test]
    fn screen_to_ground_known_camera() {
        // Camera at (0, 10, 10) looking at origin, up = (0,1,0).
        // Screen center should map roughly to near the target (origin area).
        let view = look_at_test([0.0, 10.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let proj = perspective_test(std::f32::consts::FRAC_PI_4, 800.0 / 600.0, 0.1, 500.0);
        let vp = mat4_mul_test(proj, view);
        let inv_vp = mat4_inverse(vp).expect("VP should be invertible");

        let result = screen_to_ground_raw(400.0, 300.0, 800.0, 600.0, &inv_vp);
        let (wx, wz) = result.expect("should hit ground");

        // Center of screen should hit near origin.
        assert!(wx.abs() < 2.0, "expected world_x near 0, got {wx}");
        assert!(wz.abs() < 2.0, "expected world_z near 0, got {wz}");
    }

    #[test]
    fn parallel_ray_returns_none() {
        // Identity matrix means camera at origin looking along -Z at y=0.
        // A horizontal ray won't hit y=0 if the origin is on the plane.
        // Actually with identity the near/far points both have y=0, so
        // dir.y = 0, which means parallel. Let's test that explicitly.
        let identity: [[f32; 4]; 4] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        // With identity inv_vp, the near point at ndc_y=0 has world y=0,
        // and the far point also has y=0, so ray is parallel to ground.
        let result = screen_to_ground_raw(400.0, 300.0, 800.0, 600.0, &identity);
        // dir.y = far.y - near.y = 0 - 0 = 0 => None (parallel) OR
        // near.y = 0 so t = 0 and it works. Let's check:
        // near = unproject(0, 0, 0, 1) => (0, 0, 0) after divide
        // far  = unproject(0, 0, 1, 1) => (0, 0, 1) after divide
        // dir = (0, 0, 1), dir.y = 0 => None
        assert!(result.is_none(), "horizontal ray should return None");
    }

    // -- Test helpers: minimal matrix math for constructing test cameras --

    fn build_test_inv_vp() -> [[f32; 4]; 4] {
        let view = look_at_test([0.0, 10.0, 0.0], [0.0, 0.0, -10.0], [0.0, 1.0, 0.0]);
        let proj = perspective_test(std::f32::consts::FRAC_PI_4, 800.0 / 600.0, 0.1, 500.0);
        let vp = mat4_mul_test(proj, view);
        mat4_inverse(vp).expect("VP should be invertible")
    }

    fn look_at_test(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
        let f = normalize_test(sub_test(target, eye));
        let s = normalize_test(cross_test(f, up));
        let u = cross_test(s, f);
        [
            [s[0], u[0], -f[0], 0.0],
            [s[1], u[1], -f[1], 0.0],
            [s[2], u[2], -f[2], 0.0],
            [-dot_test(s, eye), -dot_test(u, eye), dot_test(f, eye), 1.0],
        ]
    }

    fn perspective_test(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
        let f = 1.0 / (fov_y / 2.0).tan();
        let r = 1.0 / (near - far);
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far * r, -1.0],
            [0.0, 0.0, near * far * r, 0.0],
        ]
    }

    fn mat4_mul_test(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
        let mut out = [[0.0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                out[col][row] = a[0][row] * b[col][0]
                    + a[1][row] * b[col][1]
                    + a[2][row] * b[col][2]
                    + a[3][row] * b[col][3];
            }
        }
        out
    }

    /// 4x4 matrix inverse using cofactor expansion. Returns `None` if
    /// the determinant is near zero.
    fn mat4_inverse(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
        // Flatten to row-major for easier indexing.
        let mut a = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                a[row * 4 + col] = m[col][row];
            }
        }

        let mut inv = [0.0f32; 16];

        inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15]
            + a[9] * a[7] * a[14]
            + a[13] * a[6] * a[11]
            - a[13] * a[7] * a[10];

        inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15]
            - a[8] * a[7] * a[14]
            - a[12] * a[6] * a[11]
            + a[12] * a[7] * a[10];

        inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15]
            + a[8] * a[7] * a[13]
            + a[12] * a[5] * a[11]
            - a[12] * a[7] * a[9];

        inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14]
            - a[8] * a[6] * a[13]
            - a[12] * a[5] * a[10]
            + a[12] * a[6] * a[9];

        inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15]
            - a[9] * a[3] * a[14]
            - a[13] * a[2] * a[11]
            + a[13] * a[3] * a[10];

        inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15]
            + a[8] * a[3] * a[14]
            + a[12] * a[2] * a[11]
            - a[12] * a[3] * a[10];

        inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15]
            - a[8] * a[3] * a[13]
            - a[12] * a[1] * a[11]
            + a[12] * a[3] * a[9];

        inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14]
            + a[8] * a[2] * a[13]
            + a[12] * a[1] * a[10]
            - a[12] * a[2] * a[9];

        inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15]
            + a[5] * a[3] * a[14]
            + a[13] * a[2] * a[7]
            - a[13] * a[3] * a[6];

        inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15]
            - a[4] * a[3] * a[14]
            - a[12] * a[2] * a[7]
            + a[12] * a[3] * a[6];

        inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15]
            + a[4] * a[3] * a[13]
            + a[12] * a[1] * a[7]
            - a[12] * a[3] * a[5];

        inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14]
            - a[4] * a[2] * a[13]
            - a[12] * a[1] * a[6]
            + a[12] * a[2] * a[5];

        inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11]
            - a[5] * a[3] * a[10]
            - a[9] * a[2] * a[7]
            + a[9] * a[3] * a[6];

        inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11]
            + a[4] * a[3] * a[10]
            + a[8] * a[2] * a[7]
            - a[8] * a[3] * a[6];

        inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11]
            - a[4] * a[3] * a[9]
            - a[8] * a[1] * a[7]
            + a[8] * a[3] * a[5];

        inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10]
            + a[4] * a[2] * a[9]
            + a[8] * a[1] * a[6]
            - a[8] * a[2] * a[5];

        let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
        if det.abs() < 1e-10 {
            return None;
        }
        let inv_det = 1.0 / det;

        let mut result = [[0.0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                result[col][row] = inv[row * 4 + col] * inv_det;
            }
        }
        Some(result)
    }

    fn sub_test(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn cross_test(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    fn dot_test(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn normalize_test(v: [f32; 3]) -> [f32; 3] {
        let len = dot_test(v, v).sqrt();
        if len < 1e-10 {
            return [0.0; 3];
        }
        [v[0] / len, v[1] / len, v[2] / len]
    }
}
