use super::*;
use proptest::prelude::*;

// -- Basic arithmetic tests --

#[test]
fn basic_arithmetic() {
    let a = SimFloat::from_int(3);
    let b = SimFloat::from_int(2);
    assert_eq!((a + b), SimFloat::from_int(5));
    assert_eq!((a - b), SimFloat::from_int(1));
    assert_eq!((a * b), SimFloat::from_int(6));
    assert_eq!((a / b), SimFloat::from_ratio(3, 2));
}

#[test]
fn division_precision() {
    let a = SimFloat::from_int(1);
    let b = SimFloat::from_int(3);
    let result = a / b;
    // 1/3 ≈ 0.333... — check within 1 ULP of raw
    let expected = SimFloat::from_f64(1.0 / 3.0);
    assert!((result.raw() - expected.raw()).abs() <= 1);
}

#[test]
fn negation() {
    let a = SimFloat::from_int(5);
    assert_eq!(-a, SimFloat::from_int(-5));
    assert_eq!(-SimFloat::ZERO, SimFloat::ZERO);
}

#[test]
fn assign_operators() {
    let mut a = SimFloat::from_int(10);
    a += SimFloat::from_int(5);
    assert_eq!(a, SimFloat::from_int(15));
    a -= SimFloat::from_int(3);
    assert_eq!(a, SimFloat::from_int(12));
    a *= SimFloat::from_int(2);
    assert_eq!(a, SimFloat::from_int(24));
    a /= SimFloat::from_int(4);
    assert_eq!(a, SimFloat::from_int(6));
}

#[test]
fn scalar_mul_div() {
    let a = SimFloat::from_int(7);
    assert_eq!(a * 3, SimFloat::from_int(21));
    assert_eq!(3 * a, SimFloat::from_int(21));
    assert_eq!(a / 2, SimFloat::from_ratio(7, 2));
}

#[test]
fn constants() {
    assert_eq!(SimFloat::ZERO.to_f64(), 0.0);
    assert_eq!(SimFloat::ONE.to_f64(), 1.0);
    assert_eq!(SimFloat::NEG_ONE.to_f64(), -1.0);
    assert_eq!(SimFloat::TWO.to_f64(), 2.0);
    assert!((SimFloat::HALF.to_f64() - 0.5).abs() < 1e-9);
}

#[test]
fn from_ratio() {
    let half = SimFloat::from_ratio(1, 2);
    assert!((half.to_f64() - 0.5).abs() < 1e-9);
    let third = SimFloat::from_ratio(1, 3);
    assert!((third.to_f64() - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn conversions() {
    let pi = std::f64::consts::PI;
    let v = SimFloat::from_f64(pi);
    assert!((v.to_f64() - pi).abs() < 1e-9);
    assert!((v.to_f32() - pi as f32).abs() < 1e-4);

    let v2 = SimFloat::from_f32(2.5);
    assert!((v2.to_f64() - 2.5).abs() < 1e-6);
}

#[test]
fn min_max_clamp() {
    let a = SimFloat::from_int(5);
    let b = SimFloat::from_int(3);
    assert_eq!(a.min(b), b);
    assert_eq!(a.max(b), a);
    assert_eq!(SimFloat::from_int(10).clamp(b, a), a);
    assert_eq!(SimFloat::from_int(1).clamp(b, a), b);
    assert_eq!(SimFloat::from_int(4).clamp(b, a), SimFloat::from_int(4));
}

#[test]
fn floor_ceil_round() {
    let v = SimFloat::from_f64(3.7);
    assert_eq!(v.floor(), SimFloat::from_int(3));
    assert_eq!(v.ceil(), SimFloat::from_int(4));
    assert_eq!(v.round(), SimFloat::from_int(4));

    let neg = SimFloat::from_f64(-2.3);
    assert_eq!(neg.floor(), SimFloat::from_int(-3));
    assert_eq!(neg.ceil(), SimFloat::from_int(-2));
    assert_eq!(neg.round(), SimFloat::from_int(-2));
}

#[test]
fn signum() {
    assert_eq!(SimFloat::from_int(5).signum(), SimFloat::ONE);
    assert_eq!(SimFloat::from_int(-3).signum(), SimFloat::NEG_ONE);
    assert_eq!(SimFloat::ZERO.signum(), SimFloat::ZERO);
}

#[test]
fn lerp() {
    let a = SimFloat::from_int(0);
    let b = SimFloat::from_int(10);
    assert_eq!(a.lerp(b, SimFloat::ZERO), a);
    assert_eq!(a.lerp(b, SimFloat::ONE), b);
    assert_eq!(a.lerp(b, SimFloat::HALF), SimFloat::from_int(5));
}

#[test]
fn ordering() {
    let vals: Vec<SimFloat> = (-5..=5).map(SimFloat::from_int).collect();
    for w in vals.windows(2) {
        assert!(w[0] < w[1]);
    }
}

#[test]
fn determinism() {
    let a = SimFloat::from_int(7);
    let b = SimFloat::from_int(3);
    let r1 = a * b + SimFloat::from_int(1);
    let r2 = a * b + SimFloat::from_int(1);
    assert_eq!(r1.raw(), r2.raw());
}

#[test]
fn display() {
    let v = SimFloat::from_int(42);
    assert_eq!(format!("{v}"), "42.000000");
}

// -- Property-based tests --

// Use a limited range to avoid overflow in addition/multiplication chains.
const RANGE: i32 = 10_000;

fn arb_simfloat() -> impl Strategy<Value = SimFloat> {
    (-RANGE..=RANGE).prop_map(SimFloat::from_int)
}

fn arb_simfloat_nonzero() -> impl Strategy<Value = SimFloat> {
    (1..=RANGE).prop_map(SimFloat::from_int)
}

fn arb_simfloat_frac() -> impl Strategy<Value = SimFloat> {
    (-RANGE * 1000..=RANGE * 1000).prop_map(|n| SimFloat::from_ratio(n, 1000))
}

fn arb_angle() -> impl Strategy<Value = SimFloat> {
    // Angles in [-2*TAU, 2*TAU] with fractional precision
    (-25_132..=25_132i32).prop_map(|n| SimFloat::from_ratio(n, 4000))
}

proptest! {
    #[test]
    fn prop_add_commutative(a in arb_simfloat(), b in arb_simfloat()) {
        prop_assert_eq!(a + b, b + a);
    }

    #[test]
    fn prop_add_associative(a in arb_simfloat(), b in arb_simfloat(), c in arb_simfloat()) {
        prop_assert_eq!((a + b) + c, a + (b + c));
    }

    #[test]
    fn prop_mul_commutative(a in arb_simfloat(), b in arb_simfloat()) {
        prop_assert_eq!(a * b, b * a);
    }

    #[test]
    fn prop_add_identity(a in arb_simfloat()) {
        prop_assert_eq!(a + SimFloat::ZERO, a);
    }

    #[test]
    fn prop_mul_identity(a in arb_simfloat()) {
        prop_assert_eq!(a * SimFloat::ONE, a);
    }

    #[test]
    fn prop_additive_inverse(a in arb_simfloat()) {
        prop_assert_eq!(a + (-a), SimFloat::ZERO);
    }

    #[test]
    fn prop_sub_self_is_zero(a in arb_simfloat()) {
        prop_assert_eq!(a - a, SimFloat::ZERO);
    }

    #[test]
    fn prop_div_self_is_one(a in arb_simfloat_nonzero()) {
        prop_assert_eq!(a / a, SimFloat::ONE);
    }

    #[test]
    fn prop_neg_neg_identity(a in arb_simfloat()) {
        prop_assert_eq!(-(-a), a);
    }

    #[test]
    fn prop_mul_zero(a in arb_simfloat()) {
        prop_assert_eq!(a * SimFloat::ZERO, SimFloat::ZERO);
    }

    #[test]
    fn prop_div_inverse(a in arb_simfloat_frac(), b in arb_simfloat_nonzero()) {
        // (a * b) / b should be within 1 ULP of a due to rounding
        let result = (a * b) / b;
        let diff = (result.raw() - a.raw()).abs();
        prop_assert!(diff <= 1, "div inverse failed: a={a}, b={b}, result={result}, diff={diff}");
    }

    #[test]
    fn prop_abs_non_negative(a in arb_simfloat()) {
        prop_assert!(a.abs().raw() >= 0);
    }

    #[test]
    fn prop_abs_identity(a in arb_simfloat()) {
        prop_assert_eq!(a.abs(), (-a).abs());
    }

    #[test]
    fn prop_ordering_consistent(a in arb_simfloat(), b in arb_simfloat()) {
        // If a > b then b < a (and vice versa)
        if a > b {
            prop_assert!(b < a);
        } else if a < b {
            prop_assert!(b > a);
        } else {
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn prop_sin_bounded(angle in arb_angle()) {
        let s = angle.sin();
        prop_assert!(s.to_f64() >= -1.0 - 1e-6 && s.to_f64() <= 1.0 + 1e-6,
            "sin out of range: sin({}) = {}", angle.to_f64(), s.to_f64());
    }

    #[test]
    fn prop_cos_bounded(angle in arb_angle()) {
        let c = angle.cos();
        prop_assert!(c.to_f64() >= -1.0 - 1e-6 && c.to_f64() <= 1.0 + 1e-6,
            "cos out of range: cos({}) = {}", angle.to_f64(), c.to_f64());
    }

    #[test]
    fn prop_sin_cos_pythagorean(angle in arb_angle()) {
        let s = angle.sin();
        let c = angle.cos();
        let sum = (s * s + c * c).to_f64();
        prop_assert!((sum - 1.0).abs() < 1e-4,
            "sin^2 + cos^2 = {} for angle {}", sum, angle.to_f64());
    }

    #[test]
    fn prop_sin_vs_f64(angle in arb_angle()) {
        let sim = angle.sin().to_f64();
        let reference = angle.to_f64().sin();
        prop_assert!((sim - reference).abs() < 5e-5,
            "sin({}) = {} (expected {})", angle.to_f64(), sim, reference);
    }

    #[test]
    fn prop_cos_vs_f64(angle in arb_angle()) {
        let sim = angle.cos().to_f64();
        let reference = angle.to_f64().cos();
        prop_assert!((sim - reference).abs() < 5e-5,
            "cos({}) = {} (expected {})", angle.to_f64(), sim, reference);
    }

    #[test]
    fn prop_sqrt_vs_f64(val in 0..=RANGE) {
        let v = SimFloat::from_int(val);
        let sim = v.sqrt().to_f64();
        let reference = (val as f64).sqrt();
        prop_assert!((sim - reference).abs() < 1e-4,
            "sqrt({}) = {} (expected {})", val, sim, reference);
    }

    #[test]
    fn prop_sqrt_squared(val in 1..=RANGE) {
        let v = SimFloat::from_int(val);
        let s = v.sqrt();
        let back = (s * s).to_f64();
        prop_assert!((back - val as f64).abs() < 1e-3,
            "sqrt({})^2 = {} (expected {})", val, back, val);
    }

    #[test]
    fn prop_atan2_vs_f64(
        y in (-RANGE..=RANGE).prop_map(SimFloat::from_int),
        x in (-RANGE..=RANGE).prop_map(SimFloat::from_int),
    ) {
        prop_assume!(x.raw() != 0 || y.raw() != 0);
        let sim = SimFloat::atan2(y, x).to_f64();
        let reference = (y.to_f64()).atan2(x.to_f64());
        // atan2 with CORDIC: allow ~1e-4 tolerance
        prop_assert!((sim - reference).abs() < 5e-4,
            "atan2({}, {}) = {} (expected {})", y.to_f64(), x.to_f64(), sim, reference);
    }
}

// -- Trig & math unit tests --

#[test]
fn pi_constants() {
    let eps = 1e-9;
    assert!((SimFloat::PI.to_f64() - std::f64::consts::PI).abs() < eps);
    assert!((SimFloat::TAU.to_f64() - std::f64::consts::TAU).abs() < eps);
    assert!((SimFloat::FRAC_PI_2.to_f64() - std::f64::consts::FRAC_PI_2).abs() < eps);
    assert!((SimFloat::FRAC_PI_4.to_f64() - std::f64::consts::FRAC_PI_4).abs() < eps);
}

#[test]
fn sin_known_values() {
    let eps = 5e-5;
    // sin(0) = 0
    assert!(SimFloat::ZERO.sin().to_f64().abs() < eps);
    // sin(PI/2) = 1
    assert!((SimFloat::FRAC_PI_2.sin().to_f64() - 1.0).abs() < eps);
    // sin(PI) = 0
    assert!(SimFloat::PI.sin().to_f64().abs() < eps);
    // sin(3*PI/2) = -1
    let three_pi_half = SimFloat::PI + SimFloat::FRAC_PI_2;
    assert!((three_pi_half.sin().to_f64() + 1.0).abs() < eps);
    // sin(-PI/2) = -1
    assert!((-SimFloat::FRAC_PI_2).sin().to_f64() + 1.0 < eps);
}

#[test]
fn cos_known_values() {
    let eps = 5e-5;
    // cos(0) = 1
    assert!((SimFloat::ZERO.cos().to_f64() - 1.0).abs() < eps);
    // cos(PI/2) = 0
    assert!(SimFloat::FRAC_PI_2.cos().to_f64().abs() < eps);
    // cos(PI) = -1
    assert!((SimFloat::PI.cos().to_f64() + 1.0).abs() < eps);
}

#[test]
fn sqrt_known_values() {
    let eps = 1e-4;
    // sqrt(0) = 0
    assert_eq!(SimFloat::ZERO.sqrt(), SimFloat::ZERO);
    // sqrt(1) = 1
    assert!((SimFloat::ONE.sqrt().to_f64() - 1.0).abs() < eps);
    // sqrt(4) = 2
    assert!((SimFloat::from_int(4).sqrt().to_f64() - 2.0).abs() < eps);
    // sqrt(2) ~= 1.41421356
    assert!((SimFloat::TWO.sqrt().to_f64() - std::f64::consts::SQRT_2).abs() < eps);
    // sqrt(negative) = 0
    assert_eq!(SimFloat::from_int(-5).sqrt(), SimFloat::ZERO);
}

#[test]
fn sqrt_fractional() {
    let eps = 1e-4;
    // sqrt(0.25) = 0.5
    let quarter = SimFloat::from_ratio(1, 4);
    assert!((quarter.sqrt().to_f64() - 0.5).abs() < eps);
}

#[test]
fn atan2_known_values() {
    let eps = 5e-4;
    // atan2(0, 1) = 0
    assert!(
        SimFloat::atan2(SimFloat::ZERO, SimFloat::ONE)
            .to_f64()
            .abs()
            < eps
    );
    // atan2(1, 0) = PI/2
    assert!(
        (SimFloat::atan2(SimFloat::ONE, SimFloat::ZERO).to_f64() - std::f64::consts::FRAC_PI_2)
            .abs()
            < eps
    );
    // atan2(0, -1) = PI
    assert!(
        (SimFloat::atan2(SimFloat::ZERO, SimFloat::NEG_ONE).to_f64() - std::f64::consts::PI)
            .abs()
            < eps
    );
    // atan2(-1, 0) = -PI/2
    assert!(
        (SimFloat::atan2(SimFloat::NEG_ONE, SimFloat::ZERO).to_f64()
            + std::f64::consts::FRAC_PI_2)
            .abs()
            < eps
    );
    // atan2(0, 0) = 0
    assert_eq!(
        SimFloat::atan2(SimFloat::ZERO, SimFloat::ZERO),
        SimFloat::ZERO
    );
    // atan2(1, 1) = PI/4
    assert!(
        (SimFloat::atan2(SimFloat::ONE, SimFloat::ONE).to_f64() - std::f64::consts::FRAC_PI_4)
            .abs()
            < eps
    );
}

#[test]
fn sin_cos_determinism() {
    // Same input must always produce the exact same raw bits
    let angle = SimFloat::from_f64(1.2345);
    let s1 = angle.sin().raw();
    let s2 = angle.sin().raw();
    assert_eq!(s1, s2);
    let c1 = angle.cos().raw();
    let c2 = angle.cos().raw();
    assert_eq!(c1, c2);
}

#[test]
fn sqrt_determinism() {
    let v = SimFloat::from_f64(7.77);
    let r1 = v.sqrt().raw();
    let r2 = v.sqrt().raw();
    assert_eq!(r1, r2);
}

#[test]
fn atan2_determinism() {
    let y = SimFloat::from_f64(3.0);
    let x = SimFloat::from_f64(4.0);
    let a1 = SimFloat::atan2(y, x).raw();
    let a2 = SimFloat::atan2(y, x).raw();
    assert_eq!(a1, a2);
}
