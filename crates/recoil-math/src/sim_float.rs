use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Deterministic 32.32 fixed-point number for simulation math.
///
/// Stores values as i64 with 32 fractional bits, giving a range of
/// roughly ±2 billion integer part with 1/(2^32) ≈ 2.3e-10 precision.
/// All arithmetic is purely integer-based — no platform-dependent
/// floating-point results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimFloat(i64);

const SHIFT: u32 = 32;
const SCALE: i64 = 1i64 << SHIFT; // 4_294_967_296

impl SimFloat {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(SCALE);
    pub const NEG_ONE: Self = Self(-SCALE);
    pub const HALF: Self = Self(SCALE / 2);
    pub const TWO: Self = Self(SCALE * 2);
    pub const MAX: Self = Self(i64::MAX);
    pub const MIN: Self = Self(i64::MIN + 1); // avoid negation overflow

    /// Construct from raw fixed-point bits.
    #[inline]
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// Access the raw fixed-point bits.
    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Construct from an integer value (exact).
    #[inline]
    pub const fn from_int(n: i32) -> Self {
        Self((n as i64) << SHIFT)
    }

    /// Construct from a fraction `numerator / denominator` (exact for small values).
    #[inline]
    pub const fn from_ratio(num: i32, den: i32) -> Self {
        Self(((num as i64) << SHIFT) / den as i64)
    }

    /// Approximate conversion from f32 — for rendering boundary ONLY, not sim code.
    #[inline]
    pub fn from_f32(v: f32) -> Self {
        Self((v as f64 * SCALE as f64) as i64)
    }

    /// Approximate conversion from f64 — for test/data loading ONLY, not sim code.
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        Self((v * SCALE as f64) as i64)
    }

    /// Convert to f32 for rendering. Lossy.
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / SCALE as f32
    }

    /// Convert to f64 for display/debugging. Lossy.
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }

    /// Absolute value.
    #[inline]
    pub const fn abs(self) -> Self {
        if self.0 < 0 {
            Self(-self.0)
        } else {
            self
        }
    }

    /// Returns the minimum of two values.
    #[inline]
    pub const fn min(self, other: Self) -> Self {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    /// Returns the maximum of two values.
    #[inline]
    pub const fn max(self, other: Self) -> Self {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    /// Clamp value to [min, max].
    #[inline]
    pub const fn clamp(self, min: Self, max: Self) -> Self {
        self.max(min).min(max)
    }

    /// Linear interpolation: self + (other - self) * t, where t is in [0, 1].
    #[inline]
    pub fn lerp(self, other: Self, t: Self) -> Self {
        self + (other - self) * t
    }

    /// Returns the sign: -1, 0, or 1.
    #[inline]
    pub const fn signum(self) -> Self {
        if self.0 > 0 {
            Self::ONE
        } else if self.0 < 0 {
            Self::NEG_ONE
        } else {
            Self::ZERO
        }
    }

    /// Returns the floor (integer part, rounded toward negative infinity).
    #[inline]
    pub const fn floor(self) -> Self {
        // Arithmetic right shift rounds toward -inf, then shift back.
        Self((self.0 >> SHIFT) << SHIFT)
    }

    /// Returns the ceiling (integer part, rounded toward positive infinity).
    #[inline]
    pub const fn ceil(self) -> Self {
        let floor = (self.0 >> SHIFT) << SHIFT;
        if floor == self.0 {
            self
        } else {
            Self(floor + SCALE)
        }
    }

    /// Round to nearest integer (half rounds away from zero).
    #[inline]
    pub const fn round(self) -> Self {
        if self.0 >= 0 {
            // floor(self + 0.5)
            Self(((self.0 + (SCALE / 2)) >> SHIFT) << SHIFT)
        } else {
            // ceil(self - 0.5)
            let shifted = self.0 - (SCALE / 2);
            let floor = (shifted >> SHIFT) << SHIFT;
            if floor == shifted {
                Self(floor)
            } else {
                Self(floor + SCALE)
            }
        }
    }
}

// -- Arithmetic operators --

impl Add for SimFloat {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0.wrapping_add(rhs.0))
    }
}

impl Sub for SimFloat {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0.wrapping_sub(rhs.0))
    }
}

impl Mul for SimFloat {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        // Widen to i128 to avoid overflow, then shift back.
        Self(((self.0 as i128 * rhs.0 as i128) >> SHIFT) as i64)
    }
}

impl Div for SimFloat {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        assert!(rhs.0 != 0, "SimFloat division by zero");
        // Widen dividend, then divide.
        Self((((self.0 as i128) << SHIFT) / rhs.0 as i128) as i64)
    }
}

impl Neg for SimFloat {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl AddAssign for SimFloat {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for SimFloat {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl MulAssign for SimFloat {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl DivAssign for SimFloat {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}

impl std::fmt::Display for SimFloat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.6}", self.to_f64())
    }
}

impl Default for SimFloat {
    fn default() -> Self {
        Self::ZERO
    }
}

// -- Scalar multiplication (SimFloat * i32 and i32 * SimFloat) --

impl Mul<i32> for SimFloat {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: i32) -> Self {
        Self(self.0 * rhs as i64)
    }
}

impl Mul<SimFloat> for i32 {
    type Output = SimFloat;
    #[inline]
    fn mul(self, rhs: SimFloat) -> SimFloat {
        SimFloat(self as i64 * rhs.0)
    }
}

impl Div<i32> for SimFloat {
    type Output = Self;
    #[inline]
    fn div(self, rhs: i32) -> Self {
        assert!(rhs != 0, "SimFloat division by zero");
        Self(self.0 / rhs as i64)
    }
}

#[cfg(test)]
mod tests {
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
    }
}
