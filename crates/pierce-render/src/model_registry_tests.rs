use super::*;

#[test]
fn default_mesh_is_octahedron() {
    let registry = ModelRegistry::new();
    let model = registry.get(999);
    // Octahedron: 8 faces * 3 verts = 24
    assert_eq!(model.vertices.len(), 24);
    assert_eq!(model.indices.len(), 24);
}

#[test]
fn unknown_type_returns_default() {
    let registry = ModelRegistry::new();
    let a = registry.get(0);
    let b = registry.get(42);
    // Both should be the default octahedron
    assert_eq!(a.vertices.len(), b.vertices.len());
    assert_eq!(a.indices.len(), b.indices.len());
}

#[test]
fn loaded_model_returned_for_known_type() {
    let mut registry = ModelRegistry::new();
    let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1
";
    registry.load_model(7, obj).unwrap();
    let model = registry.get(7);
    assert_eq!(model.vertices.len(), 3);
    assert_eq!(model.indices.len(), 3);
}

#[test]
fn load_replaces_existing() {
    let mut registry = ModelRegistry::new();
    let tri = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1
";
    registry.load_model(1, tri).unwrap();
    assert_eq!(registry.get(1).vertices.len(), 3);

    // Load a different model for the same ID
    let quad = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1 4//1
";
    registry.load_model(1, quad).unwrap();
    assert_eq!(registry.get(1).vertices.len(), 4);
}
