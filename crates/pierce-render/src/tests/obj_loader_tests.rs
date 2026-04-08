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
