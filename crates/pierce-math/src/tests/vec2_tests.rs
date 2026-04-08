use super::*;

fn v2(x: i32, y: i32) -> SimVec2 {
    SimVec2::new(SimFloat::from_int(x), SimFloat::from_int(y))
}

#[test]
fn vec2_add_sub() {
    let a = v2(1, 2);
    let b = v2(3, 4);
    let sum = a + b;
    assert_eq!(sum.x, SimFloat::from_int(4));
    assert_eq!(sum.y, SimFloat::from_int(6));

    let diff = a - b;
    assert_eq!(diff.x, SimFloat::from_int(-2));
    assert_eq!(diff.y, SimFloat::from_int(-2));
}

#[test]
fn vec2_neg() {
    let a = v2(3, -5);
    let neg_a = -a;
    assert_eq!(neg_a.x, SimFloat::from_int(-3));
    assert_eq!(neg_a.y, SimFloat::from_int(5));
}

#[test]
fn vec2_scalar_mul_div() {
    let a = v2(2, 4);
    let s = SimFloat::from_int(3);

    let scaled = a * s;
    assert_eq!(scaled.x, SimFloat::from_int(6));
    assert_eq!(scaled.y, SimFloat::from_int(12));

    let scaled2 = s * a;
    assert_eq!(scaled2, scaled);

    let halved = a / SimFloat::from_int(2);
    assert_eq!(halved.x, SimFloat::from_int(1));
    assert_eq!(halved.y, SimFloat::from_int(2));
}

#[test]
fn vec2_dot() {
    let a = v2(1, 2);
    let b = v2(3, 4);
    // 1*3 + 2*4 = 11
    assert_eq!(a.dot(b), SimFloat::from_int(11));
}

#[test]
fn vec2_length_squared() {
    let a = v2(3, 4);
    // 9 + 16 = 25
    assert_eq!(a.length_squared(), SimFloat::from_int(25));
}

#[test]
fn vec2_distance_squared() {
    let a = v2(1, 2);
    let b = v2(4, 6);
    // (3^2 + 4^2) = 25
    assert_eq!(a.distance_squared(b), SimFloat::from_int(25));
}

#[test]
fn vec2_assign_ops() {
    let mut a = v2(1, 2);
    a += v2(3, 4);
    assert_eq!(a, v2(4, 6));
    a -= v2(1, 1);
    assert_eq!(a, v2(3, 5));
}

#[test]
fn vec2_zero_constant() {
    assert_eq!(SimVec2::ZERO.x, SimFloat::ZERO);
    assert_eq!(SimVec2::ZERO.y, SimFloat::ZERO);
}

#[test]
fn vec2_normalize_zero_returns_zero() {
    let z = SimVec2::ZERO.normalize();
    assert_eq!(z, SimVec2::ZERO);
}

#[test]
fn vec2_default() {
    assert_eq!(SimVec2::default(), SimVec2::ZERO);
}

/// Compare dot product against f64 reference.
#[test]
fn vec2_dot_vs_f64() {
    let a = SimVec2::new(SimFloat::from_f64(1.5), SimFloat::from_f64(2.5));
    let b = SimVec2::new(SimFloat::from_f64(3.0), SimFloat::from_f64(4.0));
    let dot = a.dot(b);
    let expected = 1.5 * 3.0 + 2.5 * 4.0; // 14.5
    assert!((dot.to_f64() - expected).abs() < 1e-6);
}

/// Compare distance_squared against f64 reference.
#[test]
fn vec2_distance_squared_vs_f64() {
    let a = SimVec2::new(SimFloat::from_f64(1.5), SimFloat::from_f64(2.5));
    let b = SimVec2::new(SimFloat::from_f64(4.5), SimFloat::from_f64(6.5));
    let ds = a.distance_squared(b);
    let dx = 4.5 - 1.5;
    let dy = 6.5 - 2.5;
    let expected = dx * dx + dy * dy; // 9 + 16 = 25
    assert!((ds.to_f64() - expected).abs() < 1e-6);
}
