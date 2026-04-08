use super::*;

/// Build a minimal valid .s3o byte buffer with a single piece containing
/// the given vertices and indices.
fn build_s3o(
    verts: &[[f32; 8]], // [xpos, ypos, zpos, xn, yn, zn, u, v]
    tri_indices: &[u32],
    piece_offset: [f32; 3],
) -> Vec<u8> {
    let mut buf = Vec::new();

    // --- Header (68 bytes) ---
    buf.extend_from_slice(b"Spring unit\0"); // magic (12)
    buf.extend_from_slice(&0u32.to_le_bytes()); // version
    buf.extend_from_slice(&10.0f32.to_le_bytes()); // radius
    buf.extend_from_slice(&5.0f32.to_le_bytes()); // height
    buf.extend_from_slice(&0.0f32.to_le_bytes()); // midx
    buf.extend_from_slice(&0.0f32.to_le_bytes()); // midy
    buf.extend_from_slice(&0.0f32.to_le_bytes()); // midz

    // root_piece_offset — right after header
    let root_offset = HEADER_SIZE as u32;
    buf.extend_from_slice(&root_offset.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // collision_data_offset
    buf.extend_from_slice(&0u32.to_le_bytes()); // texture1
    buf.extend_from_slice(&0u32.to_le_bytes()); // texture2

    assert_eq!(buf.len(), HEADER_SIZE);

    // --- Piece header (52 bytes) ---
    let piece_start = buf.len();

    // We'll place name string right after the piece header, then vertices,
    // then indices.
    let name_offset = (piece_start + PIECE_HEADER_SIZE) as u32;
    let name_bytes = b"root\0";

    let vertices_offset = name_offset + name_bytes.len() as u32;
    let vertex_data_len = verts.len() * VERTEX_STANDARD_SIZE;
    let indices_offset = vertices_offset + vertex_data_len as u32;

    buf.extend_from_slice(&name_offset.to_le_bytes()); // name_offset
    buf.extend_from_slice(&0u32.to_le_bytes()); // num_children
    buf.extend_from_slice(&0u32.to_le_bytes()); // children_offset
    buf.extend_from_slice(&(verts.len() as u32).to_le_bytes()); // num_vertices
    buf.extend_from_slice(&vertices_offset.to_le_bytes()); // vertices_offset
    buf.extend_from_slice(&0u32.to_le_bytes()); // vertex_type (standard)
    buf.extend_from_slice(&0u32.to_le_bytes()); // primitive_type (triangles)
    buf.extend_from_slice(&(tri_indices.len() as u32).to_le_bytes()); // num_indices
    buf.extend_from_slice(&indices_offset.to_le_bytes()); // indices_offset
    buf.extend_from_slice(&0u32.to_le_bytes()); // collision_data_offset
    buf.extend_from_slice(&piece_offset[0].to_le_bytes()); // xoffset
    buf.extend_from_slice(&piece_offset[1].to_le_bytes()); // yoffset
    buf.extend_from_slice(&piece_offset[2].to_le_bytes()); // zoffset

    assert_eq!(buf.len() - piece_start, PIECE_HEADER_SIZE);

    // --- Name ---
    buf.extend_from_slice(name_bytes);

    // --- Vertices ---
    assert_eq!(buf.len(), vertices_offset as usize);
    for v in verts {
        for &f in v {
            buf.extend_from_slice(&f.to_le_bytes());
        }
    }

    // --- Indices ---
    assert_eq!(buf.len(), indices_offset as usize);
    for &idx in tri_indices {
        buf.extend_from_slice(&idx.to_le_bytes());
    }

    buf
}

#[test]
fn parse_single_triangle() {
    let verts: [[f32; 8]; 3] = [
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    ];
    let indices = [0u32, 1, 2];

    let data = build_s3o(&verts, &indices, [0.0, 0.0, 0.0]);
    let (parsed_verts, parsed_indices) = load_s3o(&data).unwrap();

    assert_eq!(parsed_verts.len(), 3);
    assert_eq!(parsed_indices.len(), 3);

    // Check positions.
    assert_eq!(parsed_verts[0].position, [0.0, 0.0, 0.0]);
    assert_eq!(parsed_verts[1].position, [1.0, 0.0, 0.0]);
    assert_eq!(parsed_verts[2].position, [0.0, 0.0, 1.0]);

    // Check normals.
    for v in &parsed_verts {
        assert_eq!(v.normal, [0.0, 1.0, 0.0]);
    }

    // Check color is default grey.
    for v in &parsed_verts {
        assert_eq!(v.color, [0.7, 0.7, 0.7]);
    }

    // Check indices.
    assert_eq!(parsed_indices, vec![0u16, 1, 2]);
}

#[test]
fn piece_offset_applied_to_positions() {
    let verts: [[f32; 8]; 1] = [[1.0, 2.0, 3.0, 0.0, 1.0, 0.0, 0.0, 0.0]];
    let indices = [0u32];

    let data = build_s3o(&verts, &indices, [10.0, 20.0, 30.0]);
    let (parsed_verts, _) = load_s3o(&data).unwrap();

    assert_eq!(parsed_verts.len(), 1);
    assert_eq!(parsed_verts[0].position, [11.0, 22.0, 33.0]);
}

#[test]
fn invalid_magic_returns_error() {
    let mut data = vec![0u8; HEADER_SIZE];
    data[..7].copy_from_slice(b"INVALID");
    let result = load_s3o(&data);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("magic"), "error should mention magic: {err}");
}

#[test]
fn empty_piece_no_crash() {
    let data = build_s3o(&[], &[], [0.0, 0.0, 0.0]);
    let (verts, indices) = load_s3o(&data).unwrap();
    assert!(verts.is_empty());
    assert!(indices.is_empty());
}

#[test]
fn vertex_count_matches() {
    let verts: [[f32; 8]; 4] = [
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    ];
    let indices = [0u32, 1, 2, 0, 2, 3];

    let data = build_s3o(&verts, &indices, [0.0, 0.0, 0.0]);
    let (parsed_verts, parsed_indices) = load_s3o(&data).unwrap();

    assert_eq!(parsed_verts.len(), 4);
    assert_eq!(parsed_indices.len(), 6);
}

#[test]
fn file_too_small_returns_error() {
    let data = vec![0u8; 10];
    let result = load_s3o(&data);
    assert!(result.is_err());
}

#[test]
fn unsupported_version_returns_error() {
    let mut data = vec![0u8; HEADER_SIZE];
    data[..12].copy_from_slice(b"Spring unit\0");
    // Set version to 1.
    data[12..16].copy_from_slice(&1u32.to_le_bytes());
    let result = load_s3o(&data);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("version"),
        "error should mention version: {err}"
    );
}

// -----------------------------------------------------------------------
// Piece tree loading tests
// -----------------------------------------------------------------------

#[test]
fn tree_single_piece_preserves_local_space() {
    let verts: [[f32; 8]; 1] = [[1.0, 2.0, 3.0, 0.0, 1.0, 0.0, 0.0, 0.0]];
    let indices = [0u32];

    let data = build_s3o(&verts, &indices, [10.0, 20.0, 30.0]);
    let tree = load_s3o_tree(&data).unwrap();

    assert_eq!(tree.pieces.len(), 1);
    assert_eq!(tree.pieces[0].name, "root");
    assert_eq!(tree.pieces[0].local_offset, [10.0, 20.0, 30.0]);

    // Vertices should be in piece-local space (no parent offset baked in).
    assert_eq!(tree.vertices[0].position, [1.0, 2.0, 3.0]);
}

#[test]
fn tree_empty_piece() {
    let data = build_s3o(&[], &[], [0.0, 0.0, 0.0]);
    let tree = load_s3o_tree(&data).unwrap();
    assert_eq!(tree.pieces.len(), 1);
    assert!(tree.vertices.is_empty());
    assert!(tree.indices.is_empty());
}

#[test]
fn tree_piece_name_extracted() {
    let verts: [[f32; 8]; 1] = [[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]];
    let data = build_s3o(&verts, &[0u32], [0.0, 0.0, 0.0]);
    let tree = load_s3o_tree(&data).unwrap();
    assert_eq!(tree.pieces[0].name, "root");
}

#[test]
fn tree_vertex_and_index_ranges() {
    let verts: [[f32; 8]; 3] = [
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    ];
    let indices = [0u32, 1, 2];

    let data = build_s3o(&verts, &indices, [0.0, 0.0, 0.0]);
    let tree = load_s3o_tree(&data).unwrap();

    assert_eq!(tree.pieces[0].vertex_range, 0..3);
    assert_eq!(tree.pieces[0].index_range, 0..3);
}
