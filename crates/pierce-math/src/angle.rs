//! Full-circle u32 angle type.
//!
//! `Angle(u32)` represents angles where `0` = 0°, `u32::MAX` ≈ 360°.
//! Addition and subtraction wrap automatically (no normalize needed).
//! Sin/cos via 1024-entry lookup table with linear interpolation.
//!
//! This replaces `SimFloat` for angles — eliminates atan2, CORDIC,
//! and the sin/cos LUT indirection that SimFloat uses.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Sub, SubAssign, Neg};

use crate::SimFloat;

/// Full-circle angle: 0 = 0°, 2^32 = 360°.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Angle(pub u32);

// --- Constants ---

impl Angle {
    pub const ZERO: Angle = Angle(0);
    /// 90° = 2^30
    pub const QUARTER: Angle = Angle(1 << 30);
    /// 180° = 2^31
    pub const HALF: Angle = Angle(1 << 31);
    /// 360° (wraps to 0)
    pub const FULL: Angle = Angle(0);

    /// Create from radians (SimFloat).
    pub fn from_radians(rad: SimFloat) -> Self {
        // 2π radians = 2^32 units.
        // angle = rad * 2^32 / (2π) = rad * 2^31 / π
        let raw = ((rad.raw() as i128 * (1i128 << 31)) / SimFloat::PI.raw() as i128) as i64;
        Angle(raw as u32)
    }

    /// Convert to radians (SimFloat).
    pub fn to_radians(self) -> SimFloat {
        // rad = angle * 2π / 2^32 = angle * π / 2^31
        let raw = (self.0 as i64 as i128 * SimFloat::PI.raw() as i128) >> 31;
        SimFloat::from_raw(raw as i64)
    }

    /// Convert to f32 radians.
    pub fn to_f32(self) -> f32 {
        ((self.0 as f64 / (u32::MAX as f64 + 1.0)) * std::f64::consts::TAU) as f32
    }

    /// Create from degrees.
    pub fn from_degrees(deg: f64) -> Self {
        Angle(((deg / 360.0) * (u32::MAX as f64 + 1.0)) as u32)
    }

    /// Signed difference: how far to turn from self to other.
    /// Positive = counter-clockwise, negative = clockwise.
    /// Result is in range [-HALF, HALF).
    #[inline]
    pub fn signed_diff(self, other: Angle) -> i32 {
        (other.0.wrapping_sub(self.0)) as i32
    }

    /// Absolute angle difference (always positive, ≤ HALF).
    #[inline]
    pub fn abs_diff(self, other: Angle) -> u32 {
        let d = self.signed_diff(other);
        d.unsigned_abs()
    }

    /// atan2(y, x) returning an Angle.
    /// Uses the standard quadrant logic with a fast atan approximation.
    pub fn atan2(y: SimFloat, x: SimFloat) -> Self {
        // Convert to f64 for the atan2, then back to Angle.
        // This is fast (hardware instruction) and deterministic (same binary).
        let y_f = y.to_f32() as f64;
        let x_f = x.to_f32() as f64;
        let rad = libm::atan2(y_f, x_f);
        // rad is in (-π, π]. Convert to u32.
        // angle = (rad / (2π)) * 2^32
        let normalized = rad / (2.0 * std::f64::consts::PI); // (-0.5, 0.5]
        let u = ((normalized + 1.0) * (1u64 << 32) as f64) as u64;
        Angle(u as u32)
    }

    /// Sin of this angle, returned as SimFloat.
    /// Uses 1024-entry LUT with linear interpolation.
    #[inline]
    pub fn sin(self) -> SimFloat {
        sin_lut(self.0)
    }

    /// Cos of this angle, returned as SimFloat.
    #[inline]
    pub fn cos(self) -> SimFloat {
        sin_lut(self.0.wrapping_add(1 << 30)) // cos(x) = sin(x + 90°)
    }

    /// Sin and cos together (one LUT lookup each).
    #[inline]
    pub fn sincos(self) -> (SimFloat, SimFloat) {
        (self.sin(), self.cos())
    }
}

// --- Operators (wrapping) ---

impl Add for Angle {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self { Angle(self.0.wrapping_add(rhs.0)) }
}

impl AddAssign for Angle {
    #[inline]
    fn add_assign(&mut self, rhs: Self) { self.0 = self.0.wrapping_add(rhs.0); }
}

impl Sub for Angle {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self { Angle(self.0.wrapping_sub(rhs.0)) }
}

impl SubAssign for Angle {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) { self.0 = self.0.wrapping_sub(rhs.0); }
}

impl Neg for Angle {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self { Angle(self.0.wrapping_neg()) }
}


// --- Sin LUT ---

/// 1024-entry sin table covering [0, π/2] as Q0.32 fixed-point (u32).
/// Generated at compile time.
const SIN_TABLE_SIZE: usize = 1024;
const SIN_TABLE: [u32; SIN_TABLE_SIZE + 1] = generate_sin_table();

const fn generate_sin_table() -> [u32; SIN_TABLE_SIZE + 1] {
    let mut table = [0u32; SIN_TABLE_SIZE + 1];
    let mut i = 0;
    while i <= SIN_TABLE_SIZE {
        // sin(i * π/2 / 1024) as Q0.32
        // We compute in f64 at compile time.
        // Use a Taylor series approximation since const fn can't call libm.
        let x = (i as f64) * std::f64::consts::FRAC_PI_2 / (SIN_TABLE_SIZE as f64);
        let s = const_sin(x);
        table[i] = (s * (u32::MAX as f64)) as u32;
        i += 1;
    }
    table
}

/// Compile-time sin via Taylor series (enough terms for f64 precision).
const fn const_sin(x: f64) -> f64 {
    let x2 = x * x;
    let x3 = x2 * x;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;
    let x11 = x9 * x2;
    x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0 + x9 / 362880.0 - x11 / 39916800.0
}

/// Look up sin from the table. Input is u32 angle (full circle = 2^32).
/// Returns SimFloat in [-1, 1].
#[inline]
fn sin_lut(angle: u32) -> SimFloat {
    // Quadrant: top 2 bits.
    let quadrant = angle >> 30;
    // Strip quadrant bits, leaving a 30-bit position within the quadrant.
    let pos = angle & 0x3FFFFFFF;

    // For quadrants 1 and 3, mirror the index (descending half of sine).
    let pos = if quadrant & 1 != 0 { 0x3FFFFFFF - pos } else { pos };

    // Index into 1024-entry table: top 10 bits of pos.
    let idx = (pos >> 20) as usize;
    // Fractional part for interpolation: bottom 20 bits.
    let frac = pos & 0xFFFFF;

    let a = SIN_TABLE[idx] as u64;
    let b = SIN_TABLE[if idx < SIN_TABLE_SIZE { idx + 1 } else { idx }] as u64;

    // Linear interpolation: a + (b - a) * frac / 2^20
    let interp = if b >= a {
        a + (((b - a) * frac as u64) >> 20)
    } else {
        a - (((a - b) * frac as u64) >> 20)
    };

    // interp is Q0.32 (0 = 0.0, u32::MAX ≈ 1.0).
    // SimFloat Q32.32: 1.0 = 1 << 32 = 0x100000000.
    // Q0.32 value maps directly to the fractional part of Q32.32.
    let sf_raw = interp as i64;

    // Negate for quadrants 2 and 3 (bottom half of sine wave).
    if quadrant >= 2 {
        SimFloat::from_raw(-sf_raw)
    } else {
        SimFloat::from_raw(sf_raw)
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_add() {
        let a = Angle(u32::MAX - 10);
        let b = Angle(20);
        assert_eq!((a + b).0, 9);
    }

    #[test]
    fn signed_diff_forward() {
        let a = Angle(0);
        let b = Angle::QUARTER; // 90°
        let diff = a.signed_diff(b);
        assert!(diff > 0);
    }

    #[test]
    fn signed_diff_backward() {
        let a = Angle::QUARTER;
        let b = Angle(0);
        let diff = a.signed_diff(b);
        assert!(diff < 0);
    }

    #[test]
    fn sin_zero_is_zero() {
        let s = Angle::ZERO.sin();
        assert!(s.to_f32().abs() < 0.01, "sin(0) = {}", s.to_f32());
    }

    #[test]
    fn sin_90_is_one() {
        let s = Angle::QUARTER.sin();
        assert!((s.to_f32() - 1.0).abs() < 0.01, "sin(90°) = {}", s.to_f32());
    }

    #[test]
    fn cos_zero_is_one() {
        let c = Angle::ZERO.cos();
        assert!((c.to_f32() - 1.0).abs() < 0.01, "cos(0) = {}", c.to_f32());
    }

    #[test]
    fn roundtrip_radians() {
        let original = SimFloat::PI / SimFloat::from_int(4); // 45°
        let angle = Angle::from_radians(original);
        let back = angle.to_radians();
        let diff = (original - back).to_f32().abs();
        assert!(diff < 0.01, "roundtrip diff = {diff}");
    }

    #[test]
    fn atan2_basic() {
        let a = Angle::atan2(SimFloat::ZERO, SimFloat::ONE); // 0°
        assert!(a.0 < 1000 || a.0 > u32::MAX - 1000, "atan2(0,1) = {}", a.0);

        let a = Angle::atan2(SimFloat::ONE, SimFloat::ZERO); // 90°
        let diff = a.abs_diff(Angle::QUARTER);
        assert!(diff < 1_000_000, "atan2(1,0) diff from 90° = {diff}");
    }
}
