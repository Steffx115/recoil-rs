//! Hierarchical piece tree for S3O models.
//!
//! Instead of flattening all pieces into a single vertex buffer with baked
//! world offsets (as the original `s3o_loader::load_s3o` does), this module
//! preserves the parent-child hierarchy so that per-piece transforms can be
//! applied at runtime by the animation system.

use crate::unit_mesh::UnitVertex;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single node in the S3O piece hierarchy.
#[derive(Debug, Clone)]
pub struct PieceNode {
    /// Piece name (extracted from the S3O name table).
    pub name: String,
    /// Local translation offset relative to the parent piece.
    pub local_offset: [f32; 3],
    /// Range of vertices owned by this piece in the shared vertex buffer.
    pub vertex_range: std::ops::Range<u32>,
    /// Range of indices owned by this piece in the shared index buffer.
    pub index_range: std::ops::Range<u32>,
    /// Indices of child pieces in the flat `S3oPieceTree::pieces` vec.
    pub children: Vec<usize>,
}

/// Transform to apply to a single piece: translation + Euler rotation.
///
/// Translation is added on top of the piece's rest `local_offset`.
/// Rotation is applied around the piece's local origin *before* translation.
/// Angles are in radians: x = heading, y = pitch, z = bank.
#[derive(Debug, Clone, Default)]
pub struct PieceTransform {
    pub translate: [f32; 3],
    pub rotate: [f32; 3],
}

/// A complete S3O model stored as a piece tree with local-space vertices.
///
/// Vertices are in *piece-local* space (not baked with parent offsets).
/// To produce a renderable buffer, call [`flatten_with_transforms`].
#[derive(Debug, Clone)]
pub struct S3oPieceTree {
    /// Flat list of pieces. Index 0 is always the root.
    pub pieces: Vec<PieceNode>,
    /// All vertices in piece-local space.
    pub vertices: Vec<UnitVertex>,
    /// All indices (referencing into `vertices`).
    pub indices: Vec<u16>,
}

// ---------------------------------------------------------------------------
// Transform application
// ---------------------------------------------------------------------------

/// Apply a 3x3 rotation matrix (Euler ZYX order) to a position vector.
fn rotate_point(p: [f32; 3], rot: [f32; 3]) -> [f32; 3] {
    let (sx, cx) = rot[0].sin_cos();
    let (sy, cy) = rot[1].sin_cos();
    let (sz, cz) = rot[2].sin_cos();

    // ZYX Euler rotation matrix.
    let r00 = cy * cz;
    let r01 = sx * sy * cz - cx * sz;
    let r02 = cx * sy * cz + sx * sz;
    let r10 = cy * sz;
    let r11 = sx * sy * sz + cx * cz;
    let r12 = cx * sy * sz - sx * cz;
    let r20 = -sy;
    let r21 = sx * cy;
    let r22 = cx * cy;

    [
        r00 * p[0] + r01 * p[1] + r02 * p[2],
        r10 * p[0] + r11 * p[1] + r12 * p[2],
        r20 * p[0] + r21 * p[1] + r22 * p[2],
    ]
}

/// Apply a rotation matrix to a normal vector (same as position rotation
/// since we use only rotation, no non-uniform scale).
fn rotate_normal(n: [f32; 3], rot: [f32; 3]) -> [f32; 3] {
    rotate_point(n, rot)
}

/// Flatten a piece tree into a single vertex/index buffer by applying
/// hierarchical transforms.
///
/// `transforms` must have one entry per piece (same length as
/// `tree.pieces`). If shorter, identity transforms are assumed for
/// missing entries.
///
/// Returns `(vertices, indices)` ready for GPU upload.
pub fn flatten_with_transforms(
    tree: &S3oPieceTree,
    transforms: &[PieceTransform],
) -> (Vec<UnitVertex>, Vec<u16>) {
    let mut out_verts = Vec::with_capacity(tree.vertices.len());
    let mut out_indices = Vec::with_capacity(tree.indices.len());

    // Compute world-space accumulated transforms for each piece.
    let mut world_offsets: Vec<[f32; 3]> = vec![[0.0; 3]; tree.pieces.len()];
    let mut world_rotations: Vec<[f32; 3]> = vec![[0.0; 3]; tree.pieces.len()];

    // BFS / DFS from root (index 0) to accumulate transforms.
    if !tree.pieces.is_empty() {
        compute_world_transforms(
            tree,
            transforms,
            0,
            [0.0; 3],
            [0.0; 3],
            &mut world_offsets,
            &mut world_rotations,
        );
    }

    // Re-map vertices: for each piece, apply its world transform to its verts.
    // We also need to remap indices since pieces may get new base vertex indices.
    let mut vertex_remap: Vec<u32> = vec![0; tree.vertices.len()];

    for (piece_idx, piece) in tree.pieces.iter().enumerate() {
        let base_out = out_verts.len() as u32;
        let world_rot = world_rotations[piece_idx];
        let world_off = world_offsets[piece_idx];

        for vi in piece.vertex_range.start..piece.vertex_range.end {
            let src = &tree.vertices[vi as usize];
            let rotated_pos = rotate_point(src.position, world_rot);
            let rotated_normal = rotate_normal(src.normal, world_rot);
            out_verts.push(UnitVertex {
                position: [
                    rotated_pos[0] + world_off[0],
                    rotated_pos[1] + world_off[1],
                    rotated_pos[2] + world_off[2],
                ],
                normal: rotated_normal,
                color: src.color,
            });
            vertex_remap[vi as usize] = base_out + (vi - piece.vertex_range.start);
        }

        for ii in piece.index_range.start..piece.index_range.end {
            let original_idx = tree.indices[ii as usize] as u32;
            out_indices.push(vertex_remap[original_idx as usize] as u16);
        }
    }

    (out_verts, out_indices)
}

/// Recursively compute world-space offset and rotation for each piece.
fn compute_world_transforms(
    tree: &S3oPieceTree,
    transforms: &[PieceTransform],
    piece_idx: usize,
    parent_offset: [f32; 3],
    parent_rotation: [f32; 3],
    world_offsets: &mut [[f32; 3]],
    world_rotations: &mut [[f32; 3]],
) {
    let piece = &tree.pieces[piece_idx];
    let anim = transforms.get(piece_idx).cloned().unwrap_or_default();

    // Rotation: parent rotation + animation rotation.
    let local_rot = [
        parent_rotation[0] + anim.rotate[0],
        parent_rotation[1] + anim.rotate[1],
        parent_rotation[2] + anim.rotate[2],
    ];

    // Translation: parent offset + (piece local offset + animation translate),
    // rotated by parent rotation.
    let local_translate = [
        piece.local_offset[0] + anim.translate[0],
        piece.local_offset[1] + anim.translate[1],
        piece.local_offset[2] + anim.translate[2],
    ];
    let rotated_translate = rotate_point(local_translate, parent_rotation);
    let world_off = [
        parent_offset[0] + rotated_translate[0],
        parent_offset[1] + rotated_translate[1],
        parent_offset[2] + rotated_translate[2],
    ];

    world_offsets[piece_idx] = world_off;
    world_rotations[piece_idx] = local_rot;

    for &child_idx in &piece.children {
        compute_world_transforms(
            tree,
            transforms,
            child_idx,
            world_off,
            local_rot,
            world_offsets,
            world_rotations,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a simple tree with one root piece.
    fn single_piece_tree() -> S3oPieceTree {
        let vertices = vec![
            UnitVertex {
                position: [1.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            UnitVertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            UnitVertex {
                position: [0.0, 0.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
        ];
        let indices = vec![0u16, 1, 2];

        S3oPieceTree {
            pieces: vec![PieceNode {
                name: "root".to_string(),
                local_offset: [0.0, 0.0, 0.0],
                vertex_range: 0..3,
                index_range: 0..3,
                children: vec![],
            }],
            vertices,
            indices,
        }
    }

    /// Helper: create a tree with root + child piece.
    fn parent_child_tree() -> S3oPieceTree {
        let vertices = vec![
            // Root piece vertices (indices 0..3)
            UnitVertex {
                position: [1.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            UnitVertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            UnitVertex {
                position: [0.0, 0.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            // Child piece vertices (indices 3..6)
            UnitVertex {
                position: [1.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            UnitVertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
            UnitVertex {
                position: [0.0, 0.0, 1.0],
                normal: [0.0, 1.0, 0.0],
                color: [0.7, 0.7, 0.7],
            },
        ];
        let indices = vec![0u16, 1, 2, 3, 4, 5];

        S3oPieceTree {
            pieces: vec![
                PieceNode {
                    name: "root".to_string(),
                    local_offset: [0.0, 0.0, 0.0],
                    vertex_range: 0..3,
                    index_range: 0..3,
                    children: vec![1],
                },
                PieceNode {
                    name: "turret".to_string(),
                    local_offset: [10.0, 5.0, 0.0],
                    vertex_range: 3..6,
                    index_range: 3..6,
                    children: vec![],
                },
            ],
            vertices,
            indices,
        }
    }

    #[test]
    fn identity_transforms_preserve_positions() {
        let tree = single_piece_tree();
        let transforms = vec![PieceTransform::default()];
        let (verts, indices) = flatten_with_transforms(&tree, &transforms);

        assert_eq!(verts.len(), 3);
        assert_eq!(indices.len(), 3);
        assert_eq!(verts[0].position, [1.0, 0.0, 0.0]);
        assert_eq!(verts[1].position, [0.0, 1.0, 0.0]);
        assert_eq!(verts[2].position, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn translation_transform_applied() {
        let tree = single_piece_tree();
        let transforms = vec![PieceTransform {
            translate: [5.0, 10.0, 15.0],
            rotate: [0.0, 0.0, 0.0],
        }];
        let (verts, _) = flatten_with_transforms(&tree, &transforms);

        assert_eq!(verts[0].position, [6.0, 10.0, 15.0]);
        assert_eq!(verts[1].position, [5.0, 11.0, 15.0]);
        assert_eq!(verts[2].position, [5.0, 10.0, 16.0]);
    }

    #[test]
    fn child_inherits_parent_offset() {
        let tree = parent_child_tree();
        let transforms = vec![PieceTransform::default(), PieceTransform::default()];
        let (verts, _) = flatten_with_transforms(&tree, &transforms);

        // Root vertices at origin.
        assert_eq!(verts[0].position, [1.0, 0.0, 0.0]);

        // Child vertices offset by [10, 5, 0].
        assert_eq!(verts[3].position, [11.0, 5.0, 0.0]);
        assert_eq!(verts[4].position, [10.0, 6.0, 0.0]);
        assert_eq!(verts[5].position, [10.0, 5.0, 1.0]);
    }

    #[test]
    fn child_animation_adds_to_rest_offset() {
        let tree = parent_child_tree();
        let transforms = vec![
            PieceTransform::default(),
            PieceTransform {
                translate: [0.0, 3.0, 0.0], // raise turret by 3
                rotate: [0.0, 0.0, 0.0],
            },
        ];
        let (verts, _) = flatten_with_transforms(&tree, &transforms);

        // Child offset = [10, 5+3, 0] = [10, 8, 0].
        assert_eq!(verts[3].position, [11.0, 8.0, 0.0]);
    }

    #[test]
    fn empty_tree_returns_empty() {
        let tree = S3oPieceTree {
            pieces: vec![],
            vertices: vec![],
            indices: vec![],
        };
        let (verts, indices) = flatten_with_transforms(&tree, &[]);
        assert!(verts.is_empty());
        assert!(indices.is_empty());
    }

    #[test]
    fn missing_transforms_default_to_identity() {
        let tree = parent_child_tree();
        // Only provide transform for root, not for child.
        let transforms = vec![PieceTransform::default()];
        let (verts, _) = flatten_with_transforms(&tree, &transforms);

        // Child should still get its rest offset applied.
        assert_eq!(verts[3].position, [11.0, 5.0, 0.0]);
    }

    #[test]
    fn indices_are_correctly_remapped() {
        let tree = parent_child_tree();
        let transforms = vec![PieceTransform::default(), PieceTransform::default()];
        let (_, indices) = flatten_with_transforms(&tree, &transforms);

        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn rotation_90_degrees_around_y() {
        let tree = single_piece_tree();
        let half_pi = std::f32::consts::FRAC_PI_2;
        let transforms = vec![PieceTransform {
            translate: [0.0, 0.0, 0.0],
            rotate: [0.0, half_pi, 0.0], // 90 degrees around Y (pitch)
        }];
        let (verts, _) = flatten_with_transforms(&tree, &transforms);

        // Vertex at [1, 0, 0] rotated 90 deg around Y should go to ~[0, 0, -1].
        let v = verts[0].position;
        assert!((v[0]).abs() < 1e-5, "x should be ~0, got {}", v[0]);
        assert!((v[1]).abs() < 1e-5, "y should be ~0, got {}", v[1]);
        assert!((v[2] + 1.0).abs() < 1e-5, "z should be ~-1, got {}", v[2]);
    }
}
