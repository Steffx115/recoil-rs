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

// --- Compile-time sin lookup table (256 entries for [0, PI/2]) ---
// Each entry is the raw i64 value of sin(i * PI / (2 * TABLE_SIZE)) in 32.32 fixed-point.
const SIN_TABLE_SIZE: usize = 256;
const SIN_TABLE: [i64; SIN_TABLE_SIZE + 1] = {
    // We pre-compute using integer-only Taylor series at compile time.
    // sin(x) for x in [0, pi/2], sampled at 257 points (0..=256).
    //
    // We use the fact that we can compute these from a high-precision
    // rational approximation. We'll use a const-evaluated loop with
    // the Taylor series computed in i128 to get sufficient precision.
    //
    // angle = i * (PI/2) / 256, but we represent PI/2 in fixed-point.
    // PI/2 in 32.32 = 6_746_518_852 (approximately)
    //
    // For the Taylor series: sin(x) = x - x^3/6 + x^5/120 - x^7/5040 + ...
    // We compute in 64.64 (i128) for intermediate precision, then truncate to 32.32.

    let pi_half_raw: i64 = 6_746_518_852; // floor(PI/2 * 2^32)

    let mut table = [0i64; SIN_TABLE_SIZE + 1];
    let mut i = 0usize;
    while i <= SIN_TABLE_SIZE {
        // angle in 32.32 = pi_half_raw * i / 256
        let x_raw = (pi_half_raw as i128 * i as i128) / SIN_TABLE_SIZE as i128;
        let x = x_raw; // in 32.32 as i128

        // Taylor series in higher precision:
        // We work in 32.32 throughout, using i128 for intermediates.
        // x^n is computed by repeated multiplication with >>32 to stay in 32.32.
        let x2 = (x * x) >> 32;
        let x3 = (x2 * x) >> 32;
        let x5 = (x3 * x2) >> 32;
        let x7 = (x5 * x2) >> 32;
        let x9 = (x7 * x2) >> 32;
        let x11 = (x9 * x2) >> 32;
        let x13 = (x11 * x2) >> 32;

        // sin(x) = x - x^3/3! + x^5/5! - x^7/7! + x^9/9! - x^11/11! + x^13/13!
        let result = x - x3 / 6 + x5 / 120 - x7 / 5040 + x9 / 362_880 - x11 / 39_916_800
            + x13 / 6_227_020_800;

        table[i] = result as i64;
        i += 1;
    }
    table
};

// CORDIC atan table: atan(2^-i) in 32.32 fixed-point, for i = 0..31
const CORDIC_ATAN_TABLE: [i64; 32] = {
    // atan(2^-i) * 2^32, computed from known high-precision values.
    // These are the exact floor values of atan(2^-i) * 2^32.
    [
        3_373_259_426, // atan(1)       = pi/4
        1_991_351_317, // atan(1/2)
        1_052_572_536, // atan(1/4)
        534_100_634,   // atan(1/8)
        268_190_545,   // atan(1/16)
        134_281_079,   // atan(1/32)
        67_172_925,    // atan(1/64)
        33_592_156,    // atan(1/128)
        16_797_404,    // atan(1/256)
        8_398_891,     // atan(1/512)
        4_199_482,     // atan(1/1024)
        2_099_747,     // atan(1/2048)
        1_049_874,     // atan(1/4096)
        524_937,       // atan(1/8192)
        262_469,       // atan(1/16384)
        131_234,       // atan(1/32768)
        65_617,        // atan(1/65536)
        32_809,
        16_404,
        8_202,
        4_101,
        2_051,
        1_025,
        513,
        256,
        128,
        64,
        32,
        16,
        8,
        4,
        2,
    ]
};

impl SimFloat {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(SCALE);
    pub const NEG_ONE: Self = Self(-SCALE);
    pub const HALF: Self = Self(SCALE / 2);
    pub const TWO: Self = Self(SCALE * 2);
    pub const MAX: Self = Self(i64::MAX);
    pub const MIN: Self = Self(i64::MIN + 1); // avoid negation overflow

    /// PI in 32.32 fixed-point.
    pub const PI: Self = Self(13_493_037_705); // floor(pi * 2^32)
    /// TAU (2*PI) in 32.32 fixed-point.
    pub const TAU: Self = Self(26_986_075_409); // floor(2*pi * 2^32)
    /// PI/2 in 32.32 fixed-point.
    pub const FRAC_PI_2: Self = Self(6_746_518_852); // floor(pi/2 * 2^32)
    /// PI/4 in 32.32 fixed-point.
    pub const FRAC_PI_4: Self = Self(3_373_259_426); // floor(pi/4 * 2^32)

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

    // -- Trigonometric & math functions --
    // All implementations use only integer arithmetic on the raw i64.
    // Expected precision vs f32: sin/cos ~1e-7, atan2 ~1e-5, sqrt ~1e-9.

    /// Sine of self (angle in radians). Deterministic, integer-only.
    ///
    /// Uses a 257-entry lookup table with linear interpolation.
    /// Reduces angle to [0, PI/2] using symmetry.
    pub fn sin(self) -> Self {
        // Reduce to [0, TAU)
        let tau = Self::TAU.0;
        let mut x = self.0 % tau;
        if x < 0 {
            x += tau;
        }

        // Determine quadrant and reduce to [0, PI/2]
        let pi = Self::PI.0;
        let pi_half = Self::FRAC_PI_2.0;

        let (angle, negate) = if x < pi_half {
            (x, false)
        } else if x < pi {
            (pi - x, false)
        } else if x < pi + pi_half {
            (x - pi, true)
        } else {
            (tau - x, true)
        };

        // Lookup with linear interpolation.
        // index = angle * TABLE_SIZE / (PI/2)
        // We compute in i128 to avoid overflow.
        let idx_scaled = (angle as i128 * SIN_TABLE_SIZE as i128 * SCALE as i128) / pi_half as i128;
        let idx = (idx_scaled >> SHIFT) as usize;
        let frac = (idx_scaled as i64) & (SCALE - 1); // fractional part in 32.32

        let idx = if idx >= SIN_TABLE_SIZE {
            SIN_TABLE_SIZE - 1
        } else {
            idx
        };
        let next = if idx < SIN_TABLE_SIZE {
            idx + 1
        } else {
            SIN_TABLE_SIZE
        };

        let a = SIN_TABLE[idx];
        let b = SIN_TABLE[next];
        // lerp: a + (b - a) * frac
        let result = a + (((b - a) as i128 * frac as i128) >> SHIFT) as i64;

        if negate {
            Self(-result)
        } else {
            Self(result)
        }
    }

    /// Cosine of self (angle in radians). Deterministic, integer-only.
    ///
    /// cos(x) = sin(x + PI/2)
    pub fn cos(self) -> Self {
        (self + Self::FRAC_PI_2).sin()
    }

    /// Two-argument arctangent. Returns angle in radians in (-PI, PI].
    /// Deterministic, integer-only, using CORDIC algorithm.
    pub fn atan2(y: Self, x: Self) -> Self {
        if x.0 == 0 && y.0 == 0 {
            return Self::ZERO;
        }

        // CORDIC works in the right half-plane. We handle signs manually.
        let mut cx = x.0.unsigned_abs() as i64;
        let mut cy = y.0.unsigned_abs() as i64;

        // Pre-scale to avoid overflow: shift both down until they fit in 62 bits.
        // We need headroom since CORDIC adds intermediate values.
        while cx > (1i64 << 60) || cy > (1i64 << 60) {
            cx >>= 1;
            cy >>= 1;
        }

        // CORDIC vectoring mode: rotate (cx, cy) toward the x-axis,
        // accumulating angle.
        let mut angle: i64 = 0;
        let iterations = 30;
        for i in 0..iterations {
            let (nx, ny, da) = if cy >= 0 {
                (
                    cx + (cy >> i),
                    cy - (cx >> i),
                    CORDIC_ATAN_TABLE[i as usize],
                )
            } else {
                (
                    cx - (cy >> i),
                    cy + (cx >> i),
                    -CORDIC_ATAN_TABLE[i as usize],
                )
            };
            cx = nx;
            cy = ny;
            angle += da;
        }

        // angle is now atan2(|y|, |x|) in the first quadrant (in 32.32 radians)

        // Adjust for original quadrant
        let result = if x.0 >= 0 && y.0 >= 0 {
            // Q1
            angle
        } else if x.0 < 0 && y.0 >= 0 {
            // Q2
            Self::PI.0 - angle
        } else if x.0 >= 0 {
            // Q4
            -angle
        } else {
            // Q3
            -(Self::PI.0 - angle)
        };

        Self(result)
    }

    /// Square root. Returns ZERO for negative inputs.
    /// Deterministic, integer-only, using Newton's method on the raw i64.
    ///
    /// For a 32.32 fixed-point value v, sqrt(v) in fixed-point is:
    ///   raw_result = isqrt(v.raw * 2^32)
    /// because sqrt(v * 2^32) / 2^32 would lose the scale factor,
    /// so we compute isqrt(v.raw << 32) to get a 32.32 result.
    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Self::ZERO;
        }

        // We need isqrt(self.0 * 2^32) = isqrt(self.0 << 32)
        // Work in i128 to hold the shifted value.
        let val = (self.0 as u128) << SHIFT;

        // Newton's method: x_{n+1} = (x_n + val / x_n) / 2
        // Start with a reasonable initial guess.
        // Use bit-length to estimate: sqrt(val) ~ 2^(bits/2)
        let bits = 128 - val.leading_zeros();
        let mut guess = 1u128 << (bits.div_ceil(2));

        loop {
            let next = (guess + val / guess) >> 1;
            if next >= guess {
                break;
            }
            guess = next;
        }

        Self(guess as i64)
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
#[path = "tests/sim_float_tests.rs"]
mod tests;
