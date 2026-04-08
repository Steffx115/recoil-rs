use super::*;

#[test]
fn shadow_uniforms_size() {
    // 2 * 64 bytes (mat4x4) + 16 bytes (vec4) = 144 bytes
    assert_eq!(std::mem::size_of::<ShadowUniforms>(), 144);
}

#[test]
fn shadow_uniforms_is_pod() {
    let u = ShadowUniforms::zeroed();
    assert_eq!(u.cascade_splits, [0.0; 4]);
}

#[test]
fn cascade_matrices_produce_valid_output() {
    let camera = Camera::default();
    let light_dir = [0.4, 0.8, 0.3];
    let uniforms = compute_cascade_matrices(&camera, light_dir);

    // Splits should match expected values (clamped to camera range).
    assert!(uniforms.cascade_splits[0] >= 0.0);
    assert!(uniforms.cascade_splits[1] > uniforms.cascade_splits[0]);
    assert!(uniforms.cascade_splits[2] > uniforms.cascade_splits[1]);

    // Matrices should not be zero.
    let zero_mat = [[0.0f32; 4]; 4];
    assert_ne!(uniforms.light_vp_0, zero_mat);
    assert_ne!(uniforms.light_vp_1, zero_mat);
}

#[test]
fn cascade_splits_clamp_to_camera_range() {
    let mut camera = Camera::default();
    camera.near = 1.0;
    camera.far = 100.0; // Far < CASCADE_SPLIT

    let uniforms = compute_cascade_matrices(&camera, [0.4, 0.8, 0.3]);

    // With far=100, split should be clamped to 100.
    assert!((uniforms.cascade_splits[1] - 100.0).abs() < 1e-5);
    assert!((uniforms.cascade_splits[2] - 100.0).abs() < 1e-5);
}

#[test]
fn mat4_inverse_identity() {
    let id = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let inv = mat4_inverse(id);
    for c in 0..4 {
        for r in 0..4 {
            let expected = if c == r { 1.0 } else { 0.0 };
            assert!(
                (inv[c][r] - expected).abs() < 1e-5,
                "mismatch at [{c}][{r}]: {} vs {expected}",
                inv[c][r]
            );
        }
    }
}

#[test]
fn mat4_inverse_roundtrip() {
    let camera = Camera::default();
    let view = camera.view_matrix();
    let inv = mat4_inverse(view);
    let product = mat4_mul(view, inv);
    for c in 0..4 {
        for r in 0..4 {
            let expected = if c == r { 1.0 } else { 0.0 };
            assert!(
                (product[c][r] - expected).abs() < 1e-4,
                "mismatch at [{c}][{r}]: {} vs {expected}",
                product[c][r]
            );
        }
    }
}

#[test]
fn ortho_projection_maps_center_to_origin() {
    let p = ortho_projection(-10.0, 10.0, -10.0, 10.0, 0.0, 100.0);
    // Center point (0, 0, -50) should map near the center of NDC.
    let pt = mat4_transform_point(&p, [0.0, 0.0, -50.0]);
    assert!(pt[0].abs() < 1e-5, "x: {}", pt[0]);
    assert!(pt[1].abs() < 1e-5, "y: {}", pt[1]);
}

#[test]
fn normalize3_unit_length() {
    let v = normalize3([3.0, 4.0, 0.0]);
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    assert!((len - 1.0).abs() < 1e-6);
}

#[test]
fn light_look_at_produces_valid_view() {
    let view = light_look_at([0.4, 0.8, 0.3]);
    // The view matrix should be orthonormal (columns have unit length).
    for col in 0..3 {
        let len = (view[col][0].powi(2) + view[col][1].powi(2) + view[col][2].powi(2)).sqrt();
        assert!((len - 1.0).abs() < 1e-4, "column {col} length = {len}");
    }
}
