use super::*;

/// Helper: create a simple tree with one root piece.
fn single_piece_tree() -> PieceTree {
    let vertices = vec![
        ModelVertex {
            position: [1.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        ModelVertex {
            position: [0.0, 1.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        ModelVertex {
            position: [0.0, 0.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
    ];
    let indices = vec![0u16, 1, 2];

    PieceTree {
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
fn parent_child_tree() -> PieceTree {
    let vertices = vec![
        // Root piece vertices (indices 0..3)
        ModelVertex {
            position: [1.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        ModelVertex {
            position: [0.0, 1.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        ModelVertex {
            position: [0.0, 0.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        // Child piece vertices (indices 3..6)
        ModelVertex {
            position: [1.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        ModelVertex {
            position: [0.0, 1.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
        ModelVertex {
            position: [0.0, 0.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.7, 0.7, 0.7],
        },
    ];
    let indices = vec![0u16, 1, 2, 3, 4, 5];

    PieceTree {
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
    let tree = PieceTree {
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
