//! CPU implementation of BatchMathOps via scalar SimFloat operations.
//!
//! This is the determinism reference — GPU results must match these exactly.
//! Uses rayon for parallel execution on CPU.

use pierce_math::{BatchMathOps, SimFloat};
use rayon::prelude::*;

/// Minimum batch size for rayon parallelism. Below this, sequential is faster.
const PAR_THRESHOLD: usize = 256;

/// CPU batch math backend.
pub struct CpuBatchMath;

impl BatchMathOps for CpuBatchMath {
    fn batch_distance_sq(
        &mut self,
        ax: &[i64],
        az: &[i64],
        bx: &[i64],
        bz: &[i64],
    ) -> Vec<i64> {
        let n = ax.len();
        debug_assert!(n == az.len() && n == bx.len() && n == bz.len());

        if n >= PAR_THRESHOLD {
            (0..n)
                .into_par_iter()
                .map(|i| {
                    let dx = SimFloat::from_raw(ax[i]) - SimFloat::from_raw(bx[i]);
                    let dz = SimFloat::from_raw(az[i]) - SimFloat::from_raw(bz[i]);
                    (dx * dx + dz * dz).raw()
                })
                .collect()
        } else {
            (0..n)
                .map(|i| {
                    let dx = SimFloat::from_raw(ax[i]) - SimFloat::from_raw(bx[i]);
                    let dz = SimFloat::from_raw(az[i]) - SimFloat::from_raw(bz[i]);
                    (dx * dx + dz * dz).raw()
                })
                .collect()
        }
    }

    fn batch_heading(&mut self, dx: &[i64], dz: &[i64]) -> Vec<i64> {
        let n = dx.len();
        debug_assert!(n == dz.len());

        if n >= PAR_THRESHOLD {
            (0..n)
                .into_par_iter()
                .map(|i| SimFloat::atan2(SimFloat::from_raw(dz[i]), SimFloat::from_raw(dx[i])).raw())
                .collect()
        } else {
            (0..n)
                .map(|i| SimFloat::atan2(SimFloat::from_raw(dz[i]), SimFloat::from_raw(dx[i])).raw())
                .collect()
        }
    }

    fn batch_sincos(&mut self, angles: &[i64]) -> (Vec<i64>, Vec<i64>) {
        let n = angles.len();

        if n >= PAR_THRESHOLD {
            let results: Vec<(i64, i64)> = (0..n)
                .into_par_iter()
                .map(|i| {
                    let a = SimFloat::from_raw(angles[i]);
                    (a.sin().raw(), a.cos().raw())
                })
                .collect();
            results.into_iter().unzip()
        } else {
            let mut sins = Vec::with_capacity(n);
            let mut coss = Vec::with_capacity(n);
            for &a in angles {
                let a = SimFloat::from_raw(a);
                sins.push(a.sin().raw());
                coss.push(a.cos().raw());
            }
            (sins, coss)
        }
    }

    fn batch_integrate(
        &mut self,
        pos_x: &mut [i64],
        pos_y: &mut [i64],
        pos_z: &mut [i64],
        vel_x: &[i64],
        vel_y: &[i64],
        vel_z: &[i64],
    ) {
        let n = pos_x.len();
        debug_assert!(n == pos_y.len() && n == pos_z.len());
        debug_assert!(n == vel_x.len() && n == vel_y.len() && n == vel_z.len());

        // Simple addition — always fast, rayon overhead not worth it for add.
        for i in 0..n {
            pos_x[i] = pos_x[i].wrapping_add(vel_x[i]);
            pos_y[i] = pos_y[i].wrapping_add(vel_y[i]);
            pos_z[i] = pos_z[i].wrapping_add(vel_z[i]);
        }
    }

    fn batch_normalize_2d(&mut self, vx: &[i64], vy: &[i64]) -> (Vec<i64>, Vec<i64>) {
        let n = vx.len();
        debug_assert!(n == vy.len());

        if n >= PAR_THRESHOLD {
            let results: Vec<(i64, i64)> = (0..n)
                .into_par_iter()
                .map(|i| {
                    let x = SimFloat::from_raw(vx[i]);
                    let y = SimFloat::from_raw(vy[i]);
                    let len_sq = x * x + y * y;
                    if len_sq <= SimFloat::ZERO {
                        (0, 0)
                    } else {
                        let len = len_sq.sqrt();
                        ((x / len).raw(), (y / len).raw())
                    }
                })
                .collect();
            results.into_iter().unzip()
        } else {
            let mut rx = Vec::with_capacity(n);
            let mut ry = Vec::with_capacity(n);
            for i in 0..n {
                let x = SimFloat::from_raw(vx[i]);
                let y = SimFloat::from_raw(vy[i]);
                let len_sq = x * x + y * y;
                if len_sq <= SimFloat::ZERO {
                    rx.push(0);
                    ry.push(0);
                } else {
                    let len = len_sq.sqrt();
                    rx.push((x / len).raw());
                    ry.push((y / len).raw());
                }
            }
            (rx, ry)
        }
    }

    fn batch_mul(&mut self, a: &[i64], b: &[i64]) -> Vec<i64> {
        let n = a.len();
        debug_assert!(n == b.len());

        (0..n)
            .map(|i| (SimFloat::from_raw(a[i]) * SimFloat::from_raw(b[i])).raw())
            .collect()
    }

    fn batch_div(&mut self, a: &[i64], b: &[i64]) -> Vec<i64> {
        let n = a.len();
        debug_assert!(n == b.len());

        (0..n)
            .map(|i| (SimFloat::from_raw(a[i]) / SimFloat::from_raw(b[i])).raw())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_sq_matches_scalar() {
        let mut batch = CpuBatchMath;

        let ax = vec![10i64 << 32, 0, 100 << 32];
        let az = vec![20i64 << 32, 0, 200 << 32];
        let bx = vec![13i64 << 32, 5 << 32, 100 << 32];
        let bz = vec![24i64 << 32, 5 << 32, 203 << 32];

        let results = batch.batch_distance_sq(&ax, &az, &bx, &bz);

        for i in 0..ax.len() {
            let dx = SimFloat::from_raw(ax[i]) - SimFloat::from_raw(bx[i]);
            let dz = SimFloat::from_raw(az[i]) - SimFloat::from_raw(bz[i]);
            let expected = (dx * dx + dz * dz).raw();
            assert_eq!(results[i], expected, "mismatch at index {i}");
        }
    }

    #[test]
    fn heading_matches_scalar() {
        let mut batch = CpuBatchMath;

        let dx = vec![1i64 << 32, 0, -(3i64 << 32)];
        let dz = vec![0, 1i64 << 32, 4i64 << 32];

        let results = batch.batch_heading(&dx, &dz);

        for i in 0..dx.len() {
            let expected =
                SimFloat::atan2(SimFloat::from_raw(dz[i]), SimFloat::from_raw(dx[i])).raw();
            assert_eq!(results[i], expected, "heading mismatch at index {i}");
        }
    }

    #[test]
    fn sincos_matches_scalar() {
        let mut batch = CpuBatchMath;

        let angles = vec![0, SimFloat::PI.raw() / 2, SimFloat::PI.raw()];
        let (sins, coss) = batch.batch_sincos(&angles);

        for i in 0..angles.len() {
            let a = SimFloat::from_raw(angles[i]);
            assert_eq!(sins[i], a.sin().raw(), "sin mismatch at index {i}");
            assert_eq!(coss[i], a.cos().raw(), "cos mismatch at index {i}");
        }
    }

    #[test]
    fn integrate_adds_correctly() {
        let mut batch = CpuBatchMath;

        let mut px = vec![10i64 << 32, 20 << 32];
        let mut py = vec![0, 0];
        let mut pz = vec![30i64 << 32, 40 << 32];
        let vx = vec![1i64 << 32, -(2i64 << 32)];
        let vy = vec![0, 0];
        let vz = vec![3i64 << 32, -(4i64 << 32)];

        batch.batch_integrate(&mut px, &mut py, &mut pz, &vx, &vy, &vz);

        assert_eq!(px[0], 11i64 << 32);
        assert_eq!(pz[0], 33i64 << 32);
        assert_eq!(px[1], 18i64 << 32);
        assert_eq!(pz[1], 36i64 << 32);
    }

    #[test]
    fn mul_matches_scalar() {
        let mut batch = CpuBatchMath;

        let a = vec![3i64 << 32, 7 << 32, -(2i64 << 32)];
        let b = vec![4i64 << 32, 3 << 32, 5i64 << 32];

        let results = batch.batch_mul(&a, &b);

        for i in 0..a.len() {
            let expected = (SimFloat::from_raw(a[i]) * SimFloat::from_raw(b[i])).raw();
            assert_eq!(results[i], expected, "mul mismatch at index {i}");
        }
    }
}
