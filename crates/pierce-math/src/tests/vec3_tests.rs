use super::*;

fn v3(x: i32, y: i32, z: i32) -> SimVec3 {
    SimVec3::new(
        SimFloat::from_int(x),
        SimFloat::from_int(y),
        SimFloat::from_int(z),
    )
}

#[test]
fn vec3_add_sub() {
    let a = v3(1, 2, 3);
    let b = v3(4, 5, 6);
    let sum = a + b;
    assert_eq!(sum.x, SimFloat::from_int(5));
    assert_eq!(sum.y, SimFloat::from_int(7));
    assert_eq!(sum.z, SimFloat::from_int(9));

    let diff = a - b;
    assert_eq!(diff.x, SimFloat::from_int(-3));
    assert_eq!(diff.y, SimFloat::from_int(-3));
    assert_eq!(diff.z, SimFloat::from_int(-3));
}

#[test]
fn vec3_neg() {
    let a = v3(3, -5, 7);
    let neg_a = -a;
    assert_eq!(neg_a.x, SimFloat::from_int(-3));
    assert_eq!(neg_a.y, SimFloat::from_int(5));
    assert_eq!(neg_a.z, SimFloat::from_int(-7));
}

#[test]
fn vec3_scalar_mul_div() {
    let a = v3(2, 4, 6);
    let s = SimFloat::from_int(3);

    let scaled = a * s;
    assert_eq!(scaled.x, SimFloat::from_int(6));
    assert_eq!(scaled.y, SimFloat::from_int(12));
    assert_eq!(scaled.z, SimFloat::from_int(18));

    let scaled2 = s * a;
    assert_eq!(scaled2, scaled);

    let halved = a / SimFloat::from_int(2);
    assert_eq!(halved.x, SimFloat::from_int(1));
    assert_eq!(halved.y, SimFloat::from_int(2));
    assert_eq!(halved.z, SimFloat::from_int(3));
}

#[test]
fn vec3_dot() {
    let a = v3(1, 2, 3);
    let b = v3(4, 5, 6);
    // 1*4 + 2*5 + 3*6 = 32
    assert_eq!(a.dot(b), SimFloat::from_int(32));
}

#[test]
fn vec3_cross() {
    let x = v3(1, 0, 0);
    let y = v3(0, 1, 0);
    let z = x.cross(y);
    assert_eq!(z, v3(0, 0, 1));

    // Anti-commutativity: a x b = -(b x a)
    assert_eq!(y.cross(x), -z);
}

#[test]
fn vec3_cross_parallel_is_zero() {
    let a = v3(2, 4, 6);
    let b = v3(1, 2, 3);
    assert_eq!(a.cross(b), SimVec3::ZERO);
}

#[test]
fn vec3_length_squared() {
    let a = v3(1, 2, 2);
    // 1 + 4 + 4 = 9
    assert_eq!(a.length_squared(), SimFloat::from_int(9));
}

#[test]
fn vec3_distance_squared() {
    let a = v3(1, 2, 3);
    let b = v3(4, 6, 3);
    // (3^2 + 4^2 + 0^2) = 25
    assert_eq!(a.distance_squared(b), SimFloat::from_int(25));
}

#[test]
fn vec3_assign_ops() {
    let mut a = v3(1, 2, 3);
    a += v3(3, 4, 5);
    assert_eq!(a, v3(4, 6, 8));
    a -= v3(1, 1, 1);
    assert_eq!(a, v3(3, 5, 7));
}

#[test]
fn vec3_zero_constant() {
    assert_eq!(SimVec3::ZERO.x, SimFloat::ZERO);
    assert_eq!(SimVec3::ZERO.y, SimFloat::ZERO);
    assert_eq!(SimVec3::ZERO.z, SimFloat::ZERO);
}

#[test]
fn vec3_normalize_zero_returns_zero() {
    let z = SimVec3::ZERO.normalize();
    assert_eq!(z, SimVec3::ZERO);
}

#[test]
fn vec3_default() {
    assert_eq!(SimVec3::default(), SimVec3::ZERO);
}

/// Compare dot product against f64 reference.
#[test]
fn vec3_dot_vs_f64() {
    let a = SimVec3::new(
        SimFloat::from_f64(1.5),
        SimFloat::from_f64(2.5),
        SimFloat::from_f64(3.5),
    );
    let b = SimVec3::new(
        SimFloat::from_f64(4.0),
        SimFloat::from_f64(5.0),
        SimFloat::from_f64(6.0),
    );
    let dot = a.dot(b);
    let expected = 1.5 * 4.0 + 2.5 * 5.0 + 3.5 * 6.0; // 6 + 12.5 + 21 = 39.5
    assert!((dot.to_f64() - expected).abs() < 1e-6);
}

/// Compare cross product against f64 reference.
#[test]
fn vec3_cross_vs_f64() {
    let a = SimVec3::new(
        SimFloat::from_f64(1.0),
        SimFloat::from_f64(2.0),
        SimFloat::from_f64(3.0),
    );
    let b = SimVec3::new(
        SimFloat::from_f64(4.0),
        SimFloat::from_f64(5.0),
        SimFloat::from_f64(6.0),
    );
    let c = a.cross(b);
    // (2*6 - 3*5, 3*4 - 1*6, 1*5 - 2*4) = (-3, 6, -3)
    assert!((c.x.to_f64() - (-3.0)).abs() < 1e-6);
    assert!((c.y.to_f64() - 6.0).abs() < 1e-6);
    assert!((c.z.to_f64() - (-3.0)).abs() < 1e-6);
}

/// Compare distance_squared against f64 reference.
#[test]
fn vec3_distance_squared_vs_f64() {
    let a = SimVec3::new(
        SimFloat::from_f64(1.5),
        SimFloat::from_f64(2.5),
        SimFloat::from_f64(3.5),
    );
    let b = SimVec3::new(
        SimFloat::from_f64(4.5),
        SimFloat::from_f64(6.5),
        SimFloat::from_f64(3.5),
    );
    let ds = a.distance_squared(b);
    let dx = 4.5 - 1.5;
    let dy = 6.5 - 2.5;
    let dz = 3.5 - 3.5;
    let expected = dx * dx + dy * dy + dz * dz; // 9 + 16 + 0 = 25
    assert!((ds.to_f64() - expected).abs() < 1e-6);
}
