//! Simple Wavefront OBJ loader for placeholder models.
//!
//! Parses a minimal subset of OBJ: vertex positions (`v`), vertex normals
//! (`vn`), and faces with position//normal indices (`f v//n ...`). Everything
//! else is ignored.

use std::path::Path;

use anyhow::{Context, Result};

use crate::unit_mesh::UnitVertex;

/// Parse a Wavefront OBJ string into vertices and indices.
///
/// Supports:
/// - `v x y z` — vertex position
/// - `vn x y z` — vertex normal
/// - `f v1//n1 v2//n2 v3//n3` — triangular faces (position//normal)
///
/// Faces with more than 3 vertices are triangulated as a fan.
/// Returns `(vertices, indices)` suitable for indexed rendering.
pub fn load_obj(data: &str) -> Result<(Vec<UnitVertex>, Vec<u16>)> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut vertices: Vec<UnitVertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    let base_color = [0.7, 0.7, 0.7];

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(keyword) = parts.next() else {
            continue;
        };

        match keyword {
            "v" => {
                let coords = parse_float3(&mut parts).context("failed to parse vertex position")?;
                positions.push(coords);
            }
            "vn" => {
                let coords = parse_float3(&mut parts).context("failed to parse vertex normal")?;
                normals.push(coords);
            }
            "f" => {
                let face_verts: Vec<(usize, usize)> = parts
                    .map(parse_face_vertex)
                    .collect::<Result<Vec<_>>>()
                    .context("failed to parse face")?;

                if face_verts.len() < 3 {
                    continue;
                }

                // Triangulate as a fan from the first vertex.
                let first_idx = emit_vertex(
                    &face_verts[0],
                    &positions,
                    &normals,
                    base_color,
                    &mut vertices,
                )?;

                let mut prev_idx = emit_vertex(
                    &face_verts[1],
                    &positions,
                    &normals,
                    base_color,
                    &mut vertices,
                )?;

                for fv in &face_verts[2..] {
                    let cur_idx = emit_vertex(fv, &positions, &normals, base_color, &mut vertices)?;
                    indices.push(first_idx);
                    indices.push(prev_idx);
                    indices.push(cur_idx);
                    prev_idx = cur_idx;
                }
            }
            _ => {
                // Ignore unknown keywords (mtllib, usemtl, s, o, g, etc.)
            }
        }
    }

    Ok((vertices, indices))
}

/// Load an OBJ file from disk.
pub fn load_obj_file(path: &Path) -> Result<(Vec<UnitVertex>, Vec<u16>)> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read OBJ file: {}", path.display()))?;
    load_obj(&data)
}

/// Parse three whitespace-separated floats.
fn parse_float3<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Result<[f32; 3]> {
    let x: f32 = parts
        .next()
        .context("missing x")?
        .parse()
        .context("invalid x")?;
    let y: f32 = parts
        .next()
        .context("missing y")?
        .parse()
        .context("invalid y")?;
    let z: f32 = parts
        .next()
        .context("missing z")?
        .parse()
        .context("invalid z")?;
    Ok([x, y, z])
}

/// Parse a face vertex token in the form `v//n` (1-indexed).
fn parse_face_vertex(token: &str) -> Result<(usize, usize)> {
    let parts: Vec<&str> = token.split('/').collect();
    anyhow::ensure!(
        parts.len() >= 3 && parts[1].is_empty(),
        "expected face format v//n, got: {token}"
    );
    let vi: usize = parts[0].parse().context("invalid position index")?;
    let ni: usize = parts[2].parse().context("invalid normal index")?;
    anyhow::ensure!(vi >= 1, "position index must be >= 1");
    anyhow::ensure!(ni >= 1, "normal index must be >= 1");
    Ok((vi, ni))
}

/// Emit a vertex and return its index.
fn emit_vertex(
    face_vert: &(usize, usize),
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    color: [f32; 3],
    vertices: &mut Vec<UnitVertex>,
) -> Result<u16> {
    let (vi, ni) = *face_vert;
    let pos = positions
        .get(vi - 1)
        .with_context(|| format!("position index {vi} out of range"))?;
    let norm = normals
        .get(ni - 1)
        .with_context(|| format!("normal index {ni} out of range"))?;

    let idx: u16 = vertices
        .len()
        .try_into()
        .context("too many vertices for u16 index")?;
    vertices.push(UnitVertex {
        position: *pos,
        normal: *norm,
        color,
    });
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX_OBJ: &str = "\
# Simple box (cube) with 8 vertices, 6 normals, 12 triangles
v -1.0 -1.0  1.0
v  1.0 -1.0  1.0
v  1.0  1.0  1.0
v -1.0  1.0  1.0
v -1.0 -1.0 -1.0
v  1.0 -1.0 -1.0
v  1.0  1.0 -1.0
v -1.0  1.0 -1.0

vn  0.0  0.0  1.0
vn  0.0  0.0 -1.0
vn  1.0  0.0  0.0
vn -1.0  0.0  0.0
vn  0.0  1.0  0.0
vn  0.0 -1.0  0.0

# Front face
f 1//1 2//1 3//1
f 1//1 3//1 4//1
# Back face
f 6//2 5//2 8//2
f 6//2 8//2 7//2
# Right face
f 2//3 6//3 7//3
f 2//3 7//3 3//3
# Left face
f 5//4 1//4 4//4
f 5//4 4//4 8//4
# Top face
f 4//5 3//5 7//5
f 4//5 7//5 8//5
# Bottom face
f 5//6 6//6 2//6
f 5//6 2//6 1//6
";

    #[test]
    fn parse_box_obj() {
        let (verts, indices) = load_obj(BOX_OBJ).unwrap();
        // 12 triangles * 3 verts = 36 vertices
        assert_eq!(verts.len(), 36);
        // 12 triangles * 3 indices = 36
        assert_eq!(indices.len(), 36);
        // All indices should be in range
        for idx in &indices {
            assert!((*idx as usize) < verts.len());
        }
    }

    #[test]
    fn empty_obj_returns_empty() {
        let (verts, indices) = load_obj("").unwrap();
        assert!(verts.is_empty());
        assert!(indices.is_empty());
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let data = "# comment\n\n# another comment\n";
        let (verts, indices) = load_obj(data).unwrap();
        assert!(verts.is_empty());
        assert!(indices.is_empty());
    }

    #[test]
    fn normals_preserved() {
        let data = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1
";
        let (verts, _) = load_obj(data).unwrap();
        assert_eq!(verts.len(), 3);
        for v in &verts {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn quad_face_triangulated() {
        let data = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1 4//1
";
        let (verts, indices) = load_obj(data).unwrap();
        // Quad = 2 triangles = 4 verts emitted, 6 indices
        assert_eq!(verts.len(), 4);
        assert_eq!(indices.len(), 6);
    }

    #[test]
    fn invalid_index_errors() {
        let data = "\
v 0.0 0.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1
";
        let result = load_obj(data);
        assert!(result.is_err());
    }
}
