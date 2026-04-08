use serde::{Deserialize, Serialize};
use std::ops::Mul;

use crate::{SimFloat, SimVec3};

/// 4x4 transformation matrix stored in column-major order.
///
/// Used at the render boundary only. Internally uses [`SimFloat`] for
/// consistency but determinism is not required for this type.
///
/// Column-major layout: `cols[col][row]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SimMat4 {
    /// Four columns, each containing four rows.
    pub cols: [[SimFloat; 4]; 4],
}

impl SimMat4 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        cols: [
            [
                SimFloat::ONE,
                SimFloat::ZERO,
                SimFloat::ZERO,
                SimFloat::ZERO,
            ],
            [
                SimFloat::ZERO,
                SimFloat::ONE,
                SimFloat::ZERO,
                SimFloat::ZERO,
            ],
            [
                SimFloat::ZERO,
                SimFloat::ZERO,
                SimFloat::ONE,
                SimFloat::ZERO,
            ],
            [
                SimFloat::ZERO,
                SimFloat::ZERO,
                SimFloat::ZERO,
                SimFloat::ONE,
            ],
        ],
    };

    /// The zero matrix.
    pub const ZERO: Self = Self {
        cols: [[SimFloat::ZERO; 4]; 4],
    };

    /// Create a translation matrix from x, y, z offsets.
    #[inline]
    pub fn from_translation(t: SimVec3) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[3][0] = t.x;
        m.cols[3][1] = t.y;
        m.cols[3][2] = t.z;
        m
    }

    /// Create a uniform or non-uniform scale matrix.
    #[inline]
    pub fn from_scale(s: SimVec3) -> Self {
        let mut m = Self::ZERO;
        m.cols[0][0] = s.x;
        m.cols[1][1] = s.y;
        m.cols[2][2] = s.z;
        m.cols[3][3] = SimFloat::ONE;
        m
    }

    /// Convert to a flat column-major `[f32; 16]` array for passing to GPU.
    #[inline]
    pub fn to_cols_array(&self) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = self.cols[col][row].to_f32();
            }
        }
        out
    }

    /// Transform a point (SimVec3) by this matrix, treating it as a
    /// homogeneous point with w=1. Returns the xyz of the result.
    #[inline]
    pub fn transform_point(&self, p: SimVec3) -> SimVec3 {
        SimVec3::new(
            self.cols[0][0] * p.x + self.cols[1][0] * p.y + self.cols[2][0] * p.z + self.cols[3][0],
            self.cols[0][1] * p.x + self.cols[1][1] * p.y + self.cols[2][1] * p.z + self.cols[3][1],
            self.cols[0][2] * p.x + self.cols[1][2] * p.y + self.cols[2][2] * p.z + self.cols[3][2],
        )
    }
}

/// Matrix multiplication: SimMat4 * SimMat4
impl Mul for SimMat4 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let mut out = Self::ZERO;
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = SimFloat::ZERO;
                for k in 0..4 {
                    sum += self.cols[k][row] * rhs.cols[col][k];
                }
                out.cols[col][row] = sum;
            }
        }
        out
    }
}

/// Matrix * Vec3 point transform (shorthand for `transform_point`).
impl Mul<SimVec3> for SimMat4 {
    type Output = SimVec3;
    #[inline]
    fn mul(self, rhs: SimVec3) -> SimVec3 {
        self.transform_point(rhs)
    }
}

impl Default for SimMat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
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
}
