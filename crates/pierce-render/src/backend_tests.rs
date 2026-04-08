use super::*;
use crate::unit_mesh::UnitVertex;

fn dummy_vertex() -> UnitVertex {
    UnitVertex {
        position: [1.0, 2.0, 3.0],
        normal: [0.0, 1.0, 0.0],
        color: [0.5, 0.5, 0.5],
    }
}

#[test]
fn null_register_mesh() {
    let mut backend = NullBackend::default();
    assert_eq!(backend.mesh_count(), 0);

    let verts = vec![dummy_vertex(); 3];
    let indices = vec![0u16, 1, 2];
    let handle = backend.register_mesh(&verts, &indices);

    assert_eq!(backend.mesh_count(), 1);
    assert_eq!(handle, MeshId(0));
}

#[test]
fn null_register_multiple_meshes() {
    let mut backend = NullBackend::default();
    let verts = vec![dummy_vertex(); 3];
    let indices = vec![0u16, 1, 2];

    let h1 = backend.register_mesh(&verts, &indices);
    let h2 = backend.register_mesh(&verts, &indices);

    assert_ne!(h1, h2);
    assert_eq!(backend.mesh_count(), 2);
}

#[test]
fn null_update_mesh() {
    let mut backend = NullBackend::default();
    let verts = vec![dummy_vertex(); 3];
    let indices = vec![0u16, 1, 2];
    let handle = backend.register_mesh(&verts, &indices);

    let new_verts = vec![dummy_vertex(); 6];
    let new_indices = vec![0u16, 1, 2, 3, 4, 5];
    backend.update_mesh(handle, &new_verts, &new_indices);

    // Count unchanged — same handle reused.
    assert_eq!(backend.mesh_count(), 1);
}

#[test]
fn null_register_texture() {
    let mut backend = NullBackend::default();
    assert_eq!(backend.texture_count(), 0);

    let data = vec![255u8; 4 * 2 * 2]; // 2x2 RGBA
    let handle = backend.register_texture(2, 2, &data);

    assert_eq!(backend.texture_count(), 1);
    assert_eq!(handle, TextureId(0));
}

#[test]
fn null_begin_end_frame() {
    let mut backend = NullBackend::default();
    assert!(!backend.frame_active());

    backend.begin_frame().expect("begin_frame should succeed");
    assert!(backend.frame_active());

    backend.end_frame();
    assert!(!backend.frame_active());
}

#[test]
fn null_resize() {
    let mut backend = NullBackend::new(800, 600);
    assert_eq!(backend.dimensions(), (800, 600));

    backend.resize(1920, 1080);
    assert_eq!(backend.dimensions(), (1920, 1080));
}

#[test]
fn null_set_camera() {
    let mut backend = NullBackend::default();
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    backend.set_camera(identity);
    assert_eq!(backend.current_camera(), identity);
}

#[test]
fn null_frame_cycle() {
    let mut backend = NullBackend::default();

    // Run several frame cycles to ensure no state corruption.
    for _ in 0..5 {
        backend.begin_frame().unwrap();
        backend.set_camera([[1.0; 4]; 4]);
        backend.end_frame();
    }
    assert!(!backend.frame_active());
}

#[test]
fn mesh_id_equality() {
    assert_eq!(MeshId(0), MeshId(0));
    assert_ne!(MeshId(0), MeshId(1));
}

#[test]
fn texture_id_equality() {
    assert_eq!(TextureId(0), TextureId(0));
    assert_ne!(TextureId(0), TextureId(1));
}
