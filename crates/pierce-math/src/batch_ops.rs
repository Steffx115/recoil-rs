//! Batch computation traits for fixed-point math operations.
//!
//! All values are raw 32.32 fixed-point i64. Implementations may execute
//! on CPU (via scalar SimFloat) or GPU (via compute shaders) but MUST
//! produce bit-identical results.
//!
//! SoA (struct-of-arrays) interface: each component is a separate slice.
//! This maps directly to GPU buffer bindings and matches the ECS gather pattern.

/// Batch computation backend for fixed-point math operations.
pub trait BatchMathOps: Send + Sync {
    /// Squared distance between N pairs of 2D points.
    ///
    /// Returns `(dx*dx + dz*dz)` for each pair, as raw 32.32 i64.
    fn batch_distance_sq(
        &mut self,
        ax: &[i64],
        az: &[i64],
        bx: &[i64],
        bz: &[i64],
    ) -> Vec<i64>;

    /// `atan2(dz, dx)` for N direction vectors.
    ///
    /// Returns angles in raw 32.32 radians, range (-PI, PI].
    /// Must match `SimFloat::atan2` bit-for-bit.
    fn batch_heading(&mut self, dx: &[i64], dz: &[i64]) -> Vec<i64>;

    /// `sin` and `cos` for N angles (raw 32.32 radians).
    ///
    /// Returns `(sin_results, cos_results)` each of length N.
    /// Must match `SimFloat::sin` / `SimFloat::cos` bit-for-bit.
    fn batch_sincos(&mut self, angles: &[i64]) -> (Vec<i64>, Vec<i64>);

    /// `pos += vel` for N 3D entities. Mutates positions in-place.
    fn batch_integrate(
        &mut self,
        pos_x: &mut [i64],
        pos_y: &mut [i64],
        pos_z: &mut [i64],
        vel_x: &[i64],
        vel_y: &[i64],
        vel_z: &[i64],
    );

    /// Normalize N 2D vectors. Returns `(result_x, result_y)`.
    ///
    /// For zero-length vectors, returns `(0, 0)`.
    /// Must match `SimVec2::normalize` bit-for-bit.
    fn batch_normalize_2d(&mut self, vx: &[i64], vy: &[i64]) -> (Vec<i64>, Vec<i64>);

    /// Fixed-point multiplication: `a * b` for N pairs.
    ///
    /// Each result = `((a as i128 * b as i128) >> 32) as i64`.
    fn batch_mul(&mut self, a: &[i64], b: &[i64]) -> Vec<i64>;

    /// Fixed-point division: `a / b` for N pairs.
    ///
    /// Each result = `(((a as i128) << 32) / b as i128) as i64`.
    fn batch_div(&mut self, a: &[i64], b: &[i64]) -> Vec<i64>;
}
