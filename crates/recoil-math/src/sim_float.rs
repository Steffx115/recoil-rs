use serde::{Deserialize, Serialize};

/// Deterministic fixed-point number for simulation math.
/// Internally stores value as i64 with a fixed shift, avoiding
/// all floating-point non-determinism across platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimFloat(i64);

const SHIFT: u32 = 16;
const SCALE: i64 = 1 << SHIFT; // 65536

impl SimFloat {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(SCALE);

    #[inline]
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }

    #[inline]
    pub const fn from_int(n: i32) -> Self {
        Self((n as i64) << SHIFT)
    }

    /// Approximate conversion from f64 — for test/data loading only, NOT sim code.
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        Self((v * SCALE as f64) as i64)
    }

    #[inline]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }

    #[inline]
    pub const fn abs(self) -> Self {
        if self.0 < 0 {
            Self(-self.0)
        } else {
            self
        }
    }
}

impl std::ops::Add for SimFloat {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for SimFloat {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Mul for SimFloat {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self(((self.0 as i128 * rhs.0 as i128) >> SHIFT) as i64)
    }
}

impl std::ops::Div for SimFloat {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        assert!(rhs.0 != 0, "SimFloat division by zero");
        Self(((self.0 as i128) << SHIFT) as i64 / rhs.0)
    }
}

impl std::ops::Neg for SimFloat {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl std::fmt::Display for SimFloat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.4}", self.to_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_arithmetic() {
        let a = SimFloat::from_int(3);
        let b = SimFloat::from_int(2);
        assert_eq!((a + b).to_f64(), 5.0);
        assert_eq!((a - b).to_f64(), 1.0);
        assert_eq!((a * b).to_f64(), 6.0);
        assert_eq!((a / b).to_f64(), 1.5);
    }

    #[test]
    fn negation() {
        let a = SimFloat::from_int(5);
        assert_eq!((-a).to_f64(), -5.0);
    }

    #[test]
    fn determinism() {
        // Same operation must always produce identical raw bits
        let a = SimFloat::from_int(7);
        let b = SimFloat::from_int(3);
        let r1 = a * b + SimFloat::from_int(1);
        let r2 = a * b + SimFloat::from_int(1);
        assert_eq!(r1.raw(), r2.raw());
    }
}
