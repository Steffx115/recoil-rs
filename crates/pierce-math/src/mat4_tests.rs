use super::*;

fn sf(v: f64) -> SimFloat {
    SimFloat::from_f64(v)
}

fn v3f(x: f64, y: f64, z: f64) -> SimVec3 {
    SimVec3::new(sf(x), sf(y), sf(z))
}

#[test]
fn identity_is_identity() {
    let m = SimMat4::IDENTITY;
    let p = v3f(1.0, 2.0, 3.0);
    let result = m * p;
    assert!((result.x.to_f64() - 1.0).abs() < 1e-6);
    assert!((result.y.to_f64() - 2.0).abs() < 1e-6);
    assert!((result.z.to_f64() - 3.0).abs() < 1e-6);
}

#[test]
fn identity_mul_identity() {
    let m = SimMat4::IDENTITY * SimMat4::IDENTITY;
    assert_eq!(m, SimMat4::IDENTITY);
}

#[test]
fn translation() {
    let t = SimMat4::from_translation(v3f(10.0, 20.0, 30.0));
    let p = v3f(1.0, 2.0, 3.0);
    let result = t * p;
    assert!((result.x.to_f64() - 11.0).abs() < 1e-6);
    assert!((result.y.to_f64() - 22.0).abs() < 1e-6);
    assert!((result.z.to_f64() - 33.0).abs() < 1e-6);
}

#[test]
fn scale() {
    let s = SimMat4::from_scale(v3f(2.0, 3.0, 4.0));
    let p = v3f(1.0, 2.0, 3.0);
    let result = s * p;
    assert!((result.x.to_f64() - 2.0).abs() < 1e-6);
    assert!((result.y.to_f64() - 6.0).abs() < 1e-6);
    assert!((result.z.to_f64() - 12.0).abs() < 1e-6);
}

#[test]
fn scale_then_translate() {
    let s = SimMat4::from_scale(v3f(2.0, 2.0, 2.0));
    let t = SimMat4::from_translation(v3f(10.0, 0.0, 0.0));
    // Apply scale first, then translate: T * S * p
    let m = t * s;
    let p = v3f(1.0, 1.0, 1.0);
    let result = m * p;
    // Scale: (2,2,2), then translate: (12, 2, 2)
    assert!((result.x.to_f64() - 12.0).abs() < 1e-6);
    assert!((result.y.to_f64() - 2.0).abs() < 1e-6);
    assert!((result.z.to_f64() - 2.0).abs() < 1e-6);
}

#[test]
fn translate_then_scale() {
    let s = SimMat4::from_scale(v3f(2.0, 2.0, 2.0));
    let t = SimMat4::from_translation(v3f(10.0, 0.0, 0.0));
    // Apply translate first, then scale: S * T * p
    let m = s * t;
    let p = v3f(1.0, 1.0, 1.0);
    let result = m * p;
    // Translate: (11,1,1), then scale: (22, 2, 2)
    assert!((result.x.to_f64() - 22.0).abs() < 1e-6);
    assert!((result.y.to_f64() - 2.0).abs() < 1e-6);
    assert!((result.z.to_f64() - 2.0).abs() < 1e-6);
}

#[test]
fn to_cols_array_identity() {
    let arr = SimMat4::IDENTITY.to_cols_array();
    #[rustfmt::skip]
    let expected: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    for (a, b) in arr.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-6, "got {a}, expected {b}");
    }
}

#[test]
fn to_cols_array_translation() {
    let t = SimMat4::from_translation(v3f(5.0, 10.0, 15.0));
    let arr = t.to_cols_array();
    // Column 3 should contain the translation
    assert!((arr[12] - 5.0).abs() < 1e-4);
    assert!((arr[13] - 10.0).abs() < 1e-4);
    assert!((arr[14] - 15.0).abs() < 1e-4);
    assert!((arr[15] - 1.0).abs() < 1e-4);
}

/// Compare matrix multiply against f64 reference.
#[test]
fn mat4_mul_vs_f64() {
    let s = SimMat4::from_scale(v3f(2.0, 3.0, 4.0));
    let t = SimMat4::from_translation(v3f(1.0, 2.0, 3.0));
    let m = t * s;
    let p = v3f(1.0, 1.0, 1.0);
    let result = m * p;

    // f64 reference: scale then translate
    // scale: (2, 3, 4), translate: (3, 5, 7)
    let ex = 2.0 + 1.0;
    let ey = 3.0 + 2.0;
    let ez = 4.0 + 3.0;
    assert!((result.x.to_f64() - ex).abs() < 1e-6);
    assert!((result.y.to_f64() - ey).abs() < 1e-6);
    assert!((result.z.to_f64() - ez).abs() < 1e-6);
}

#[test]
fn default_is_identity() {
    assert_eq!(SimMat4::default(), SimMat4::IDENTITY);
}

#[test]
fn zero_transform() {
    let p = v3f(5.0, 10.0, 15.0);
    let result = SimMat4::from_translation(SimVec3::ZERO) * p;
    assert!((result.x.to_f64() - 5.0).abs() < 1e-6);
    assert!((result.y.to_f64() - 10.0).abs() < 1e-6);
    assert!((result.z.to_f64() - 15.0).abs() < 1e-6);
}
