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
#[path = "mat4_tests.rs"]
mod tests;
