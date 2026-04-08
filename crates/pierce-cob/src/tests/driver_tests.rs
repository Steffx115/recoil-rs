use super::*;
use crate::loader::{CobFunction, CobScript};
use pierce_model::{ModelVertex, PieceNode, PieceTree};

// COB opcodes used in test scripts.
const RETURN: u32 = 0x10065000;
const PUSH_CONSTANT: u32 = 0x10021001;
const MOVE_PIECE_NOW: u32 = 0x1000B000;
const SLEEP: u32 = 0x10013000;

/// Helper: build a CobScript directly (no binary parsing needed).
fn make_script(
    pieces: &[&str],
    scripts: Vec<(&str, Vec<u32>)>,
    num_static_vars: usize,
) -> CobScript {
    CobScript {
        pieces: pieces.iter().map(|s| s.to_string()).collect(),
        scripts: scripts
            .into_iter()
            .map(|(name, code)| CobFunction {
                name: name.to_string(),
                code,
            })
            .collect(),
        num_static_vars,
    }
}

/// Build a minimal COB binary using the Spring 44-byte header format.
fn build_test_cob(
    piece_names: &[&str],
    script_names: &[&str],
    script_codes: &[Vec<u32>],
) -> Vec<u8> {
    assert_eq!(script_names.len(), script_codes.len());

    let num_pieces = piece_names.len();
    let num_scripts = script_names.len();
    let code_start: usize = 44; // COB_HEADER_SIZE

    let mut code_word_offsets = Vec::new();
    let mut word_cursor: usize = 0;
    for code in script_codes {
        code_word_offsets.push(word_cursor);
        word_cursor += code.len();
    }
    let total_code_words = word_cursor;

    let tables_start = code_start + total_code_words * 4;
    let script_code_offsets_start = tables_start;
    let script_name_offsets_start = script_code_offsets_start + num_scripts * 4;
    let piece_name_offsets_start = script_name_offsets_start + num_scripts * 4;
    let strings_start = piece_name_offsets_start + num_pieces * 4;

    let mut string_offsets = Vec::new();
    let mut cursor = strings_start;
    for &name in piece_names.iter().chain(script_names.iter()) {
        string_offsets.push(cursor);
        cursor += name.len() + 1;
    }

    let total_size = cursor;
    let mut buf = vec![0u8; total_size];

    // Header (44 bytes).
    buf[0..4].copy_from_slice(&4i32.to_le_bytes());
    buf[4..8].copy_from_slice(&(num_scripts as i32).to_le_bytes());
    buf[8..12].copy_from_slice(&(num_pieces as i32).to_le_bytes());
    buf[12..16].copy_from_slice(&(total_code_words as i32).to_le_bytes());
    buf[16..20].copy_from_slice(&0i32.to_le_bytes()); // num_static_vars
    buf[20..24].copy_from_slice(&0i32.to_le_bytes()); // unused
    buf[24..28].copy_from_slice(&(script_code_offsets_start as i32).to_le_bytes());
    buf[28..32].copy_from_slice(&(script_name_offsets_start as i32).to_le_bytes());
    buf[32..36].copy_from_slice(&(piece_name_offsets_start as i32).to_le_bytes());
    buf[36..40].copy_from_slice(&(code_start as i32).to_le_bytes());
    buf[40..44].copy_from_slice(&0i32.to_le_bytes()); // unused

    // Code data.
    let mut code_byte_cursor = code_start;
    for code in script_codes {
        for &word in code {
            buf[code_byte_cursor..code_byte_cursor + 4].copy_from_slice(&word.to_le_bytes());
            code_byte_cursor += 4;
        }
    }

    // Script code offsets (word indices relative to code_start).
    for (i, &off) in code_word_offsets.iter().enumerate() {
        let pos = script_code_offsets_start + i * 4;
        buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
    }
    // Script name offsets.
    for (i, &off) in string_offsets[num_pieces..].iter().enumerate() {
        let pos = script_name_offsets_start + i * 4;
        buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
    }
    // Piece name offsets.
    for (i, &off) in string_offsets[..num_pieces].iter().enumerate() {
        let pos = piece_name_offsets_start + i * 4;
        buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
    }

    // String data.
    let mut str_cursor = strings_start;
    for &name in piece_names.iter().chain(script_names.iter()) {
        buf[str_cursor..str_cursor + name.len()].copy_from_slice(name.as_bytes());
        buf[str_cursor + name.len()] = 0;
        str_cursor += name.len() + 1;
    }

    buf
}

/// Helper: simple piece tree with one root piece.
fn simple_tree() -> PieceTree {
    PieceTree {
        pieces: vec![PieceNode {
            name: "base".to_string(),
            local_offset: [0.0, 0.0, 0.0],
            vertex_range: 0..3,
            index_range: 0..3,
            children: vec![],
        }],
        vertices: vec![
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
        ],
        indices: vec![0, 1, 2],
    }
}

// -----------------------------------------------------------------
// Tests
// -----------------------------------------------------------------

#[test]
fn test_spawn_creates_vm() {
    let mut driver = CobAnimationDriver::new();
    let script = make_script(&["base"], vec![("Create", vec![RETURN])], 0);
    driver.scripts.insert(1, script);

    driver.spawn_unit(100, 1);
    assert!(driver.has_unit(100));
    assert!(driver.units.get(&100).unwrap().create_called);
}

#[test]
fn test_set_moving_starts_walk() {
    let mut driver = CobAnimationDriver::new();
    let script = make_script(
        &["base"],
        vec![
            ("Create", vec![RETURN]),
            ("StartMoving", vec![RETURN]),
            ("Walk", vec![SLEEP, PUSH_CONSTANT, 1000, RETURN]),
        ],
        0,
    );
    driver.scripts.insert(1, script);
    driver.spawn_unit(100, 1);

    // Tick to clear the Create thread.
    driver.tick();

    driver.set_moving(100, true);
    let inst = driver.units.get(&100).unwrap();
    assert!(inst.is_moving);
    assert!(inst.walk_started);
    // Should have StartMoving thread queued (Walk is started by StartMoving internally).
    assert!(inst.vm.threads.len() >= 1);
}

#[test]
fn test_fire_starts_script() {
    let mut driver = CobAnimationDriver::new();
    let script = make_script(
        &["base"],
        vec![
            ("Create", vec![RETURN]),
            ("FirePrimary", vec![SLEEP, PUSH_CONSTANT, 100, RETURN]),
        ],
        0,
    );
    driver.scripts.insert(1, script);
    driver.spawn_unit(100, 1);
    driver.tick(); // process Create

    driver.fire(100);
    let inst = driver.units.get(&100).unwrap();
    // FirePrimary thread should be running.
    assert!(!inst.vm.threads.is_empty());
}

#[test]
fn test_tick_advances_vm() {
    // Script: Create moves piece 0 on Y axis to 65536 (=1.0 world unit).
    // Stack: position. Inline: piece, axis.
    let create_code = vec![
        PUSH_CONSTANT,
        65536, // position = 1.0
        MOVE_PIECE_NOW,
        0, // piece 0 (inline)
        1, // axis Y (inline)
        RETURN,
    ];
    let mut driver = CobAnimationDriver::new();
    let script = make_script(&["base"], vec![("Create", create_code)], 0);
    driver.scripts.insert(1, script);

    driver.spawn_unit(100, 1);
    driver.tick();

    let transforms = driver.get_transforms(100).unwrap();
    assert_eq!(transforms.len(), 1);
    // The Y position should now be 1.0.
    assert!(
        (transforms[0].translate[1] - 1.0).abs() < 1e-4,
        "expected ~1.0, got {}",
        transforms[0].translate[1]
    );
}

#[test]
fn test_despawn_removes_unit() {
    let mut driver = CobAnimationDriver::new();
    let script = make_script(&["base"], vec![("Create", vec![RETURN])], 0);
    driver.scripts.insert(1, script);

    driver.spawn_unit(100, 1);
    assert!(driver.has_unit(100));

    driver.despawn_unit(100);
    assert!(!driver.has_unit(100));
}

#[test]
fn test_no_script_graceful() {
    let mut driver = CobAnimationDriver::new();
    // No script loaded for type 99.
    driver.spawn_unit(200, 99);
    assert!(!driver.has_unit(200));

    // These should all be no-ops, not panics.
    driver.set_moving(200, true);
    driver.fire(200);
    driver.tick();
    assert!(driver.get_transforms(200).is_none());
}

#[test]
fn test_generate_animated_mesh() {
    // Create a script that moves piece 0 up by 1 unit on Y.
    let create_code = vec![
        PUSH_CONSTANT,
        65536, // 1.0 world unit
        MOVE_PIECE_NOW,
        0, // piece 0 (inline)
        1, // axis Y (inline)
        RETURN,
    ];
    let mut driver = CobAnimationDriver::new();
    let script = make_script(&["base"], vec![("Create", create_code)], 0);
    driver.scripts.insert(1, script);

    driver.spawn_unit(100, 1);
    driver.tick();

    let tree = simple_tree();
    let (verts, indices) = driver.generate_animated_mesh(100, &tree).unwrap();

    assert_eq!(verts.len(), 3);
    assert_eq!(indices.len(), 3);

    // All vertices should have Y offset by +1.0.
    assert!(
        (verts[0].position[1] - 1.0).abs() < 1e-4,
        "expected Y ~1.0, got {}",
        verts[0].position[1]
    );
    assert!(
        (verts[1].position[1] - 2.0).abs() < 1e-4,
        "expected Y ~2.0, got {}",
        verts[1].position[1]
    );
}

#[test]
fn test_load_script_from_binary() {
    let cob_data = build_test_cob(
        &["base", "turret"],
        &["Create"],
        &[vec![0x10066]], // RETURN
    );
    let mut driver = CobAnimationDriver::new();
    driver.load_script(42, &cob_data).unwrap();
    assert!(driver.has_script(42));
    assert!(!driver.has_script(99));
}

#[test]
fn test_stop_moving_calls_stop() {
    let mut driver = CobAnimationDriver::new();
    let script = make_script(
        &["base"],
        vec![
            ("Create", vec![RETURN]),
            ("StartMoving", vec![RETURN]),
            ("Walk", vec![RETURN]),
            ("StopMoving", vec![RETURN]),
        ],
        0,
    );
    driver.scripts.insert(1, script);
    driver.spawn_unit(100, 1);
    driver.tick();

    driver.set_moving(100, true);
    let inst = driver.units.get(&100).unwrap();
    assert!(inst.is_moving);
    assert!(inst.walk_started);

    driver.set_moving(100, false);
    let inst = driver.units.get(&100).unwrap();
    assert!(!inst.is_moving);
    assert!(!inst.walk_started);
}
