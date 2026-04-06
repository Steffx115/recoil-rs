//! Parser for the Spring .s3o (Spring 3D Object) binary model format.
//!
//! The .s3o format stores hierarchical piece-based models used by the Spring
//! RTS engine. Each piece has its own vertices, indices, and a translation
//! offset relative to its parent. This loader flattens all pieces into a
//! single vertex/index buffer suitable for indexed rendering.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::unit_mesh::UnitVertex;

// ---------------------------------------------------------------------------
// Binary format constants
// ---------------------------------------------------------------------------

const S3O_MAGIC: &[u8; 12] = b"Spring unit\0";
const HEADER_SIZE: usize = 52;
const PIECE_HEADER_SIZE: usize = 52;
const VERTEX_STANDARD_SIZE: usize = 32; // 8 floats * 4 bytes
const VERTEX_TANGENT_SIZE: usize = 40; // 10 floats * 4 bytes

// ---------------------------------------------------------------------------
// Internal structs
// ---------------------------------------------------------------------------

/// Parsed .s3o file header.
struct S3oHeader {
    #[allow(dead_code)]
    version: u32,
    #[allow(dead_code)]
    radius: f32,
    #[allow(dead_code)]
    height: f32,
    #[allow(dead_code)]
    midx: f32,
    #[allow(dead_code)]
    midy: f32,
    #[allow(dead_code)]
    midz: f32,
    root_piece_offset: u32,
    #[allow(dead_code)]
    collision_data_offset: u32,
    #[allow(dead_code)]
    texture1_name_offset: u32,
    #[allow(dead_code)]
    texture2_name_offset: u32,
}

/// Parsed .s3o piece header.
struct S3oPiece {
    #[allow(dead_code)]
    name_offset: u32,
    num_children: u32,
    children_offset: u32,
    num_vertices: u32,
    vertices_offset: u32,
    vertex_type: u32,
    primitive_type: u32,
    num_indices: u32,
    indices_offset: u32,
    #[allow(dead_code)]
    collision_data_offset: u32,
    xoffset: f32,
    yoffset: f32,
    zoffset: f32,
}

// ---------------------------------------------------------------------------
// Helper: read little-endian primitives
// ---------------------------------------------------------------------------

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .context("unexpected end of file reading u32")?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(bytes))
}

fn read_f32(data: &[u8], offset: usize) -> Result<f32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .context("unexpected end of file reading f32")?
        .try_into()
        .unwrap();
    Ok(f32::from_le_bytes(bytes))
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_header(data: &[u8]) -> Result<S3oHeader> {
    ensure!(data.len() >= HEADER_SIZE, "file too small for s3o header");

    let magic = &data[0..12];
    ensure!(
        magic == S3O_MAGIC,
        "invalid s3o magic (expected \"Spring unit\\0\")"
    );

    let version = read_u32(data, 12)?;
    ensure!(version == 0, "unsupported s3o version {version}");

    Ok(S3oHeader {
        version,
        radius: read_f32(data, 16)?,
        height: read_f32(data, 20)?,
        midx: read_f32(data, 24)?,
        midy: read_f32(data, 28)?,
        midz: read_f32(data, 32)?,
        root_piece_offset: read_u32(data, 36)?,
        collision_data_offset: read_u32(data, 40)?,
        texture1_name_offset: read_u32(data, 44)?,
        texture2_name_offset: read_u32(data, 48)?,
    })
}

fn parse_piece(data: &[u8], offset: usize) -> Result<S3oPiece> {
    ensure!(
        data.len() >= offset + PIECE_HEADER_SIZE,
        "file too small for piece at offset {offset}"
    );

    Ok(S3oPiece {
        name_offset: read_u32(data, offset)?,
        num_children: read_u32(data, offset + 4)?,
        children_offset: read_u32(data, offset + 8)?,
        num_vertices: read_u32(data, offset + 12)?,
        vertices_offset: read_u32(data, offset + 16)?,
        vertex_type: read_u32(data, offset + 20)?,
        primitive_type: read_u32(data, offset + 24)?,
        num_indices: read_u32(data, offset + 28)?,
        indices_offset: read_u32(data, offset + 32)?,
        collision_data_offset: read_u32(data, offset + 36)?,
        xoffset: read_f32(data, offset + 40)?,
        yoffset: read_f32(data, offset + 44)?,
        zoffset: read_f32(data, offset + 48)?,
    })
}

/// Recursively collect vertices and indices from a piece and all its children.
///
/// `parent_offset` is the accumulated world-space translation from all
/// ancestor pieces.
fn collect_piece(
    data: &[u8],
    piece_offset: usize,
    parent_offset: [f32; 3],
    vertices: &mut Vec<UnitVertex>,
    indices: &mut Vec<u32>,
) -> Result<()> {
    let piece = parse_piece(data, piece_offset)?;

    // Accumulated offset for this piece.
    let world_offset = [
        parent_offset[0] + piece.xoffset,
        parent_offset[1] + piece.yoffset,
        parent_offset[2] + piece.zoffset,
    ];

    let base_color = [0.7f32, 0.7, 0.7];
    let vertex_stride = match piece.vertex_type {
        0 => VERTEX_STANDARD_SIZE,
        1 => VERTEX_TANGENT_SIZE,
        other => anyhow::bail!("unsupported s3o vertex type {other}"),
    };

    // Base index for this piece's vertices within the global buffer.
    let base_vertex = vertices.len() as u32;

    // Read vertices.
    let verts_start = piece.vertices_offset as usize;
    for i in 0..piece.num_vertices as usize {
        let vo = verts_start + i * vertex_stride;
        let xpos = read_f32(data, vo)?;
        let ypos = read_f32(data, vo + 4)?;
        let zpos = read_f32(data, vo + 8)?;
        let xnormal = read_f32(data, vo + 12)?;
        let ynormal = read_f32(data, vo + 16)?;
        let znormal = read_f32(data, vo + 20)?;
        // UV at vo+24, vo+28 — skipped.

        vertices.push(UnitVertex {
            position: [
                xpos + world_offset[0],
                ypos + world_offset[1],
                zpos + world_offset[2],
            ],
            normal: [xnormal, ynormal, znormal],
            color: base_color,
        });
    }

    // Read indices (u32).
    let idx_start = piece.indices_offset as usize;
    match piece.primitive_type {
        0 => {
            // Triangles
            for i in 0..piece.num_indices as usize {
                let idx = read_u32(data, idx_start + i * 4)?;
                indices.push(base_vertex + idx);
            }
        }
        1 => {
            // Triangle strips — convert to triangles.
            let mut strip_indices = Vec::with_capacity(piece.num_indices as usize);
            for i in 0..piece.num_indices as usize {
                strip_indices.push(read_u32(data, idx_start + i * 4)?);
            }
            for i in 2..strip_indices.len() {
                if i % 2 == 0 {
                    indices.push(base_vertex + strip_indices[i - 2]);
                    indices.push(base_vertex + strip_indices[i - 1]);
                    indices.push(base_vertex + strip_indices[i]);
                } else {
                    // Flip winding for odd triangles in strip.
                    indices.push(base_vertex + strip_indices[i - 1]);
                    indices.push(base_vertex + strip_indices[i - 2]);
                    indices.push(base_vertex + strip_indices[i]);
                }
            }
        }
        2 => {
            // Quads — convert to triangles (two per quad).
            ensure!(
                piece.num_indices % 4 == 0,
                "quad primitive type but index count {} not divisible by 4",
                piece.num_indices
            );
            let mut quad_indices = Vec::with_capacity(piece.num_indices as usize);
            for i in 0..piece.num_indices as usize {
                quad_indices.push(read_u32(data, idx_start + i * 4)?);
            }
            for chunk in quad_indices.chunks(4) {
                indices.push(base_vertex + chunk[0]);
                indices.push(base_vertex + chunk[1]);
                indices.push(base_vertex + chunk[2]);
                indices.push(base_vertex + chunk[0]);
                indices.push(base_vertex + chunk[2]);
                indices.push(base_vertex + chunk[3]);
            }
        }
        other => anyhow::bail!("unsupported s3o primitive type {other}"),
    }

    // Recurse into children.
    let children_start = piece.children_offset as usize;
    for i in 0..piece.num_children as usize {
        let child_offset = read_u32(data, children_start + i * 4)? as usize;
        collect_piece(data, child_offset, world_offset, vertices, indices)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a .s3o model from an in-memory byte slice.
///
/// Returns `(vertices, indices)` with all pieces flattened into a single
/// buffer. Vertex positions include the accumulated piece offsets. Indices
/// are converted to `u16`; a warning is logged and indices are truncated
/// if the total vertex count exceeds 65 535.
pub fn load_s3o(data: &[u8]) -> Result<(Vec<UnitVertex>, Vec<u16>)> {
    let header = parse_header(data)?;

    let mut vertices = Vec::new();
    let mut indices_u32 = Vec::new();

    if header.root_piece_offset > 0 || data.len() > HEADER_SIZE {
        collect_piece(
            data,
            header.root_piece_offset as usize,
            [0.0, 0.0, 0.0],
            &mut vertices,
            &mut indices_u32,
        )?;
    }

    // Convert u32 indices to u16.
    let vertex_count = vertices.len();
    if vertex_count > u16::MAX as usize {
        tracing::warn!(
            "s3o model has {vertex_count} vertices, exceeding u16 max; indices will be truncated"
        );
    }

    let indices: Vec<u16> = indices_u32
        .iter()
        .map(|&idx| {
            if idx > u16::MAX as u32 {
                tracing::warn!("s3o index {idx} exceeds u16 max, clamping to {}", u16::MAX);
                u16::MAX
            } else {
                idx as u16
            }
        })
        .collect();

    Ok((vertices, indices))
}

/// Load a .s3o model from a file on disk.
pub fn load_s3o_file(path: &Path) -> Result<(Vec<UnitVertex>, Vec<u16>)> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read s3o file: {}", path.display()))?;
    load_s3o(&data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
