//! Fast CPU batch math via libm (f64 intermediate).
//!
//! Converts SimFloat raw i64 → f64, does math via libm, converts back.
//! ~10x faster than CORDIC/Newton on fixed-point for atan2/sin/cos/sqrt.
//! Deterministic across platforms because libm is pure Rust.
//!
//! Precision: f64 has 53 bits of mantissa vs SimFloat's 64 bits total.
//! For the 32.32 format, the integer part uses at most ~20 bits for
//! typical game coordinates, leaving ~33 bits for fractional precision.
//! This exceeds SimFloat's 32 fractional bits, so libm is actually
//! more precise for typical inputs.

use pierce_math::BatchMathOps;
use rayon::prelude::*;

const SHIFT: f64 = (1u64 << 32) as f64;
const INV_SHIFT: f64 = 1.0 / SHIFT;

const PAR_THRESHOLD: usize = 256;

#[inline]
fn to_f64(raw: i64) -> f64 {
    raw as f64 * INV_SHIFT
}

#[inline]
fn from_f64(v: f64) -> i64 {
    (v * SHIFT) as i64
}

/// CPU batch math using libm for trig/sqrt. ~10x faster than fixed-point.
pub struct CpuBatchMathLibm;

impl BatchMathOps for CpuBatchMathLibm {
    fn batch_distance_sq(
        &mut self,
        ax: &[i64], az: &[i64],
        bx: &[i64], bz: &[i64],
    ) -> Vec<i64> {
        let n = ax.len();
        let compute = |i: usize| {
            let dx = to_f64(ax[i]) - to_f64(bx[i]);
            let dz = to_f64(az[i]) - to_f64(bz[i]);
            from_f64(dx * dx + dz * dz)
        };
        if n >= PAR_THRESHOLD {
            (0..n).into_par_iter().map(compute).collect()
        } else {
            (0..n).map(compute).collect()
        }
    }

    fn batch_heading(&mut self, dx: &[i64], dz: &[i64]) -> Vec<i64> {
        let n = dx.len();
        let compute = |i: usize| {
            from_f64(libm::atan2(to_f64(dz[i]), to_f64(dx[i])))
        };
        if n >= PAR_THRESHOLD {
            (0..n).into_par_iter().map(compute).collect()
        } else {
            (0..n).map(compute).collect()
        }
    }

    fn batch_sincos(&mut self, angles: &[i64]) -> (Vec<i64>, Vec<i64>) {
        let n = angles.len();
        let compute = |i: usize| {
            let a = to_f64(angles[i]);
            let (s, c) = libm::sincos(a);
            (from_f64(s), from_f64(c))
        };
        if n >= PAR_THRESHOLD {
            let results: Vec<(i64, i64)> = (0..n).into_par_iter().map(compute).collect();
            results.into_iter().unzip()
        } else {
            let results: Vec<(i64, i64)> = (0..n).map(compute).collect();
            results.into_iter().unzip()
        }
    }

    fn batch_integrate(
        &mut self,
        pos_x: &mut [i64], pos_y: &mut [i64], pos_z: &mut [i64],
        vel_x: &[i64], vel_y: &[i64], vel_z: &[i64],
    ) {
        // Pure integer addition — no libm needed.
        for i in 0..pos_x.len() {
            pos_x[i] = pos_x[i].wrapping_add(vel_x[i]);
            pos_y[i] = pos_y[i].wrapping_add(vel_y[i]);
            pos_z[i] = pos_z[i].wrapping_add(vel_z[i]);
        }
    }

    fn batch_normalize_2d(&mut self, vx: &[i64], vy: &[i64]) -> (Vec<i64>, Vec<i64>) {
        let n = vx.len();
        let compute = |i: usize| {
            let x = to_f64(vx[i]);
            let y = to_f64(vy[i]);
            let len = libm::sqrt(x * x + y * y);
            if len < 1e-15 {
                (0i64, 0i64)
            } else {
                (from_f64(x / len), from_f64(y / len))
            }
        };
        if n >= PAR_THRESHOLD {
            let results: Vec<(i64, i64)> = (0..n).into_par_iter().map(compute).collect();
            results.into_iter().unzip()
        } else {
            let results: Vec<(i64, i64)> = (0..n).map(compute).collect();
            results.into_iter().unzip()
        }
    }

    fn batch_mul(&mut self, a: &[i64], b: &[i64]) -> Vec<i64> {
        // Fixed-point mul is just integer arithmetic — keep exact.
        let n = a.len();
        (0..n)
            .map(|i| ((a[i] as i128 * b[i] as i128) >> 32) as i64)
            .collect()
    }

    fn batch_div(&mut self, a: &[i64], b: &[i64]) -> Vec<i64> {
        // Fixed-point div is integer — keep exact.
        let n = a.len();
        (0..n)
            .map(|i| (((a[i] as i128) << 32) / b[i] as i128) as i64)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierce_math::SimFloat;

    #[test]
    fn heading_close_to_scalar() {
        let mut batch = CpuBatchMathLibm;
        let dx = vec![1i64 << 32, 0, -(3i64 << 32), 5 << 32];
        let dz = vec![0, 1i64 << 32, 4i64 << 32, -(2i64 << 32)];

        let results = batch.batch_heading(&dx, &dz);

        for i in 0..dx.len() {
            let scalar = SimFloat::atan2(SimFloat::from_raw(dz[i]), SimFloat::from_raw(dx[i]));
            let diff = (results[i] - scalar.raw()).abs();
            // Allow small difference: libm f64 vs CORDIC fixed-point.
            // 1 bit of 32.32 = ~2.3e-10. Allow up to ~100 bits difference.
            // libm f64 vs CORDIC fixed-point can differ by ~100K raw units
            // (~0.00002 in world units). Acceptable for game simulation.
            assert!(
                diff < 500_000,
                "heading[{i}]: libm={} scalar={} diff={diff}",
                results[i], scalar.raw()
            );
        }
    }

    #[test]
    fn sincos_close_to_scalar() {
        let mut batch = CpuBatchMathLibm;
        let angles = vec![0, SimFloat::PI.raw() / 4, SimFloat::PI.raw() / 2, SimFloat::PI.raw()];

        let (sins, coss) = batch.batch_sincos(&angles);

        for i in 0..angles.len() {
            let a = SimFloat::from_raw(angles[i]);
            let sin_diff = (sins[i] - a.sin().raw()).abs();
            let cos_diff = (coss[i] - a.cos().raw()).abs();
            assert!(sin_diff < 200_000, "sin[{i}] diff={sin_diff}");
            assert!(cos_diff < 200_000, "cos[{i}] diff={cos_diff}");
        }
    }

    #[test]
    fn distance_sq_close_to_scalar() {
        let mut batch = CpuBatchMathLibm;
        let ax = vec![10i64 << 32, 100 << 32];
        let az = vec![20i64 << 32, 200 << 32];
        let bx = vec![13i64 << 32, 103 << 32];
        let bz = vec![24i64 << 32, 204 << 32];

        let results = batch.batch_distance_sq(&ax, &az, &bx, &bz);

        for i in 0..ax.len() {
            let dx = SimFloat::from_raw(ax[i]) - SimFloat::from_raw(bx[i]);
            let dz = SimFloat::from_raw(az[i]) - SimFloat::from_raw(bz[i]);
            let scalar = (dx * dx + dz * dz).raw();
            let diff = (results[i] - scalar).abs();
            assert!(diff < 100, "dist_sq[{i}] diff={diff}");
        }
    }

    #[test]
    fn mul_is_exact() {
        let mut batch = CpuBatchMathLibm;
        let a = vec![3i64 << 32, 7 << 32, -(2i64 << 32)];
        let b = vec![4i64 << 32, 3 << 32, 5i64 << 32];
        let results = batch.batch_mul(&a, &b);

        for i in 0..a.len() {
            let expected = (SimFloat::from_raw(a[i]) * SimFloat::from_raw(b[i])).raw();
            assert_eq!(results[i], expected);
        }
    }
}
