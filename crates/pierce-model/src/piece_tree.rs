//! Hierarchical piece tree for 3D models.
//!
//! Instead of flattening all pieces into a single vertex buffer with baked
//! world offsets, this module preserves the parent-child hierarchy so that
//! per-piece transforms can be applied at runtime by the animation system.

use crate::vertex::ModelVertex;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single node in the piece hierarchy.
#[derive(Debug, Clone)]
pub struct PieceNode {
    /// Piece name (extracted from the model's name table).
    pub name: String,
    /// Local translation offset relative to the parent piece.
    pub local_offset: [f32; 3],
    /// Range of vertices owned by this piece in the shared vertex buffer.
    pub vertex_range: std::ops::Range<u32>,
    /// Range of indices owned by this piece in the shared index buffer.
    pub index_range: std::ops::Range<u32>,
    /// Indices of child pieces in the flat `PieceTree::pieces` vec.
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

/// A complete model stored as a piece tree with local-space vertices.
///
/// Vertices are in *piece-local* space (not baked with parent offsets).
/// To produce a renderable buffer, call [`flatten_with_transforms`].
#[derive(Debug, Clone)]
pub struct PieceTree {
    /// Flat list of pieces. Index 0 is always the root.
    pub pieces: Vec<PieceNode>,
    /// All vertices in piece-local space.
    pub vertices: Vec<ModelVertex>,
    /// All indices (referencing into `vertices`).
    pub indices: Vec<u16>,
}

// ---------------------------------------------------------------------------
// Model loader trait
// ---------------------------------------------------------------------------

/// Trait for format-specific model loaders (S3O, OBJ, etc.).
pub trait ModelLoader {
    /// Load a model preserving the piece hierarchy.
    fn load_tree(data: &[u8]) -> anyhow::Result<PieceTree>
    where
        Self: Sized;

    /// Load a model as a flat vertex/index buffer (pieces baked in).
    fn load_flat(data: &[u8]) -> anyhow::Result<(Vec<ModelVertex>, Vec<u16>)>
    where
        Self: Sized;
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
    tree: &PieceTree,
    transforms: &[PieceTransform],
) -> (Vec<ModelVertex>, Vec<u16>) {
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
            out_verts.push(ModelVertex {
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
    tree: &PieceTree,
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
#[path = "tests/piece_tree_tests.rs"]
mod tests;
