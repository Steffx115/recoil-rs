use super::*;

/// Build a minimal valid COB binary for testing.
///
/// Uses the Spring 44-byte header format:
///   [0..44)   header (11 i32 fields)
///   [44..)    code area (bytecode words)
///   [after code]  script code offset table
///   [after code offsets]  script name offset table
///   [after name offsets]  piece name offset table
///   [after piece offsets]  string data (null-terminated names)
fn build_test_cob(
    piece_names: &[&str],
    script_names: &[&str],
    script_codes: &[Vec<u32>],
) -> Vec<u8> {
    assert_eq!(script_names.len(), script_codes.len());

    let num_pieces = piece_names.len();
    let num_scripts = script_names.len();
    let code_start: usize = COB_HEADER_SIZE; // 44

    // Compute total code words and per-script offsets (relative to code_start).
    let mut code_word_offsets = Vec::new();
    let mut word_cursor: usize = 0;
    for code in script_codes {
        code_word_offsets.push(word_cursor);
        word_cursor += code.len();
    }
    let total_code_words = word_cursor;

    // Tables come after the code area.
    let tables_start = code_start + total_code_words * 4;
    let script_code_offsets_start = tables_start;
    let script_name_offsets_start = script_code_offsets_start + num_scripts * 4;
    let piece_name_offsets_start = script_name_offsets_start + num_scripts * 4;
    let strings_start = piece_name_offsets_start + num_pieces * 4;

    // Compute string offsets.
    let mut string_offsets = Vec::new();
    let mut cursor = strings_start;
    for &name in piece_names.iter().chain(script_names.iter()) {
        string_offsets.push(cursor);
        cursor += name.len() + 1;
    }

    let total_size = cursor;
    let mut buf = vec![0u8; total_size];

    // Header (44 bytes = 11 i32 fields).
    buf[0..4].copy_from_slice(&4i32.to_le_bytes()); // version_sig
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

    // Code data (right after header).
    let mut code_byte_cursor = code_start;
    for code in script_codes {
        for &word in code {
            buf[code_byte_cursor..code_byte_cursor + 4].copy_from_slice(&word.to_le_bytes());
            code_byte_cursor += 4;
        }
    }

    // Script code offsets table (word indices relative to code_start).
    for (i, &off) in code_word_offsets.iter().enumerate() {
        let pos = script_code_offsets_start + i * 4;
        buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
    }

    // Script name offsets table.
    for (i, &off) in string_offsets[num_pieces..].iter().enumerate() {
        let pos = script_name_offsets_start + i * 4;
        buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
    }

    // Piece name offsets table.
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

#[test]
fn parse_minimal_cob() {
    let data = build_test_cob(
        &["base", "turret"],
        &["Create", "AimPrimary"],
        &[
            vec![0x10021, 0, 0x10066], // PUSH_CONSTANT 0, RETURN
            vec![0x10021, 1, 0x10066], // PUSH_CONSTANT 1, RETURN
        ],
    );

    let cob = parse_cob(&data).expect("parse should succeed");

    assert_eq!(cob.pieces.len(), 2);
    assert_eq!(cob.pieces[0], "base");
    assert_eq!(cob.pieces[1], "turret");

    assert_eq!(cob.scripts.len(), 2);
    assert_eq!(cob.scripts[0].name, "Create");
    assert_eq!(cob.scripts[1].name, "AimPrimary");

    assert_eq!(cob.scripts[0].code.len(), 3);
    assert_eq!(cob.scripts[0].code[0], 0x10021); // PUSH_CONSTANT
    assert_eq!(cob.scripts[0].code[2], 0x10066); // RETURN
}

#[test]
fn parse_single_piece_single_script() {
    let data = build_test_cob(
        &["body"],
        &["Create"],
        &[vec![0x10066]], // just RETURN
    );

    let cob = parse_cob(&data).unwrap();
    assert_eq!(cob.pieces.len(), 1);
    assert_eq!(cob.pieces[0], "body");
    assert_eq!(cob.scripts.len(), 1);
    assert_eq!(cob.scripts[0].name, "Create");
    assert_eq!(cob.scripts[0].code, vec![0x10066]);
}

#[test]
fn bad_version_rejected() {
    let mut data = vec![0u8; 44];
    data[0..4].copy_from_slice(&3i32.to_le_bytes()); // wrong version
    let result = parse_cob(&data);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("version"), "error: {err}");
}

#[test]
fn file_too_small_rejected() {
    let data = vec![0u8; 10];
    let result = parse_cob(&data);
    assert!(result.is_err());
}

#[test]
fn no_pieces_no_scripts() {
    let data = build_test_cob(&[], &[], &[]);
    let cob = parse_cob(&data).unwrap();
    assert!(cob.pieces.is_empty());
    assert!(cob.scripts.is_empty());
}

#[test]
fn piece_names_with_special_chars() {
    let data = build_test_cob(
        &["arm_left_01", "leg.right.02"],
        &["Walk"],
        &[vec![0x10066]],
    );
    let cob = parse_cob(&data).unwrap();
    assert_eq!(cob.pieces[0], "arm_left_01");
    assert_eq!(cob.pieces[1], "leg.right.02");
}
