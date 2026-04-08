/// A perspective camera for the RTS viewport.
///
/// All math here uses `f32` -- this is render-side code, not simulation.
pub struct Camera {
    /// Camera position in world space.
    pub eye: [f32; 3],
    /// Look-at target in world space.
    pub target: [f32; 3],
    /// Up vector.
    pub up: [f32; 3],
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Aspect ratio (width / height).
    pub aspect: f32,
    /// Near clip plane.
    pub near: f32,
    /// Far clip plane.
    pub far: f32,
}

impl Default for Camera {
    /// Default top-down RTS camera looking at the origin from above at ~60 degrees.
    fn default() -> Self {
        Self {
            eye: [32.0, 40.0, 32.0],
            target: [32.0, 0.0, 32.0],
            up: [0.0, 0.0, -1.0],
            fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 500.0,
        }
    }
}

impl Camera {
    /// Compute the view (look-at) matrix.
    pub fn view_matrix(&self) -> [[f32; 4]; 4] {
        look_at(self.eye, self.target, self.up)
    }

    /// Compute the perspective projection matrix (right-handed, zero-to-one depth).
    pub fn projection_matrix(&self) -> [[f32; 4]; 4] {
        perspective(self.fov_y, self.aspect, self.near, self.far)
    }

    /// Compute the combined view-projection matrix.
    pub fn view_projection(&self) -> [[f32; 4]; 4] {
        mat4_mul(self.projection_matrix(), self.view_matrix())
    }
}

// ---------------------------------------------------------------------------
// Manual matrix math (column-major, right-handed, depth [0,1])
// ---------------------------------------------------------------------------

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = dot3(v, v).sqrt();
    if len < 1e-10 {
        return [0.0; 3];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

/// Right-handed look-at view matrix (column-major storage).
fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize3(sub3(target, eye)); // forward
    let s = normalize3(cross(f, up)); // right
    let u = cross(s, f); // recalculated up

    // Column-major: each inner array is a column.
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot3(s, eye), -dot3(u, eye), dot3(f, eye), 1.0],
    ]
}

/// Right-handed perspective projection with depth mapped to [0, 1] (wgpu convention).
fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y / 2.0).tan();
    let range_inv = 1.0 / (near - far);

    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far * range_inv, -1.0],
        [0.0, 0.0, near * far * range_inv, 0.0],
    ]
}

/// Multiply two 4x4 column-major matrices: result = a * b.
fn mat4_mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_view_at_origin() {
        // Looking down -Z from origin should produce something close to identity rotation.
        let cam = Camera {
            eye: [0.0, 0.0, 0.0],
            target: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        };
        let v = cam.view_matrix();
        // Diagonal should be ~1 (identity rotation part).
        assert!((v[0][0] - 1.0).abs() < 1e-5);
        assert!((v[1][1] - 1.0).abs() < 1e-5);
        assert!((v[2][2] - 1.0).abs() < 1e-5);
        assert!((v[3][3] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn projection_near_far() {
        let cam = Camera::default();
        let p = cam.projection_matrix();
        // p[2][3] should be -1 for a standard perspective matrix.
        assert!((p[2][3] - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn view_projection_is_product() {
        let cam = Camera::default();
        let vp = cam.view_projection();
        let expected = mat4_mul(cam.projection_matrix(), cam.view_matrix());
        for c in 0..4 {
            for r in 0..4 {
                assert!(
                    (vp[c][r] - expected[c][r]).abs() < 1e-5,
                    "mismatch at [{c}][{r}]"
                );
            }
        }
    }
}
