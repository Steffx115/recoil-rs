use bytemuck::{Pod, Zeroable};

// ---------------------------------------------------------------------------
// Vertex type
// ---------------------------------------------------------------------------

/// Unit vertex: position + normal + color.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct UnitVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

impl UnitVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<UnitVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            // position
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            // normal
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            // color
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    };
}

// ---------------------------------------------------------------------------
// Placeholder mesh generation
// ---------------------------------------------------------------------------

/// Generate a placeholder unit mesh (octahedron / diamond shape).
///
/// Returns `(vertices, indices)`. Each face has its own vertices so normals
/// are per-face (flat shading). The base color is a neutral grey; team color
/// is applied per-instance in the shader.
pub fn generate_unit_mesh() -> (Vec<UnitVertex>, Vec<u16>) {
    // Octahedron: 6 vertices at +/- along each axis, 8 triangular faces.
    // Scale factor so units are visible in the world (CollisionRadius ~8)
    let s = 8.0f32;
    let top = [0.0f32, 1.5 * s, 0.0];
    let bottom = [0.0, -0.5 * s, 0.0]; // less below ground
    let front = [0.0, 0.5 * s, s];
    let back = [0.0, 0.5 * s, -s];
    let left = [-s, 0.5 * s, 0.0];
    let right = [s, 0.5 * s, 0.0];

    // 8 faces, each a triangle.
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 8] = [
        // Upper faces
        (top, front, right),
        (top, right, back),
        (top, back, left),
        (top, left, front),
        // Lower faces
        (bottom, right, front),
        (bottom, back, right),
        (bottom, left, back),
        (bottom, front, left),
    ];

    let base_color = [0.7, 0.7, 0.7];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(24);

    for (i, (a, b, c)) in faces.iter().enumerate() {
        let normal = face_normal(*a, *b, *c);
        let idx = (i as u16) * 3;
        vertices.push(UnitVertex {
            position: *a,
            normal,
            color: base_color,
        });
        vertices.push(UnitVertex {
            position: *b,
            normal,
            color: base_color,
        });
        vertices.push(UnitVertex {
            position: *c,
            normal,
            color: base_color,
        });
        indices.push(idx);
        indices.push(idx + 1);
        indices.push(idx + 2);
    }

    (vertices, indices)
}

/// Compute the face normal from three vertices (counter-clockwise winding).
fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-10 {
        return [0.0, 1.0, 0.0];
    }
    [n[0] / len, n[1] / len, n[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_non_empty() {
        let (verts, indices) = generate_unit_mesh();
        assert!(!verts.is_empty(), "vertices must not be empty");
        assert!(!indices.is_empty(), "indices must not be empty");
        // Octahedron: 8 faces * 3 verts = 24
        assert_eq!(verts.len(), 24);
        assert_eq!(indices.len(), 24);
    }

    #[test]
    fn normals_are_unit_length() {
        let (verts, _) = generate_unit_mesh();
        for v in &verts {
            let len = (v.normal[0].powi(2) + v.normal[1].powi(2) + v.normal[2].powi(2)).sqrt();
            assert!(
                (len - 1.0).abs() < 1e-4,
                "normal length {len} should be ~1.0"
            );
        }
    }

    #[test]
    fn indices_in_range() {
        let (verts, indices) = generate_unit_mesh();
        for idx in &indices {
            assert!((*idx as usize) < verts.len());
        }
    }
}
