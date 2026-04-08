//! Parser for the Spring COB (compiled BOS) animation bytecode format.
//!
//! COB files contain stack-based animation scripts that drive piece
//! movement, rotation, and visibility for S3O models. This module parses
//! the binary header, piece names, script names, and raw bytecode segments.

use anyhow::{ensure, Context, Result};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single script function extracted from a COB file.
#[derive(Debug, Clone)]
pub struct CobFunction {
    /// Script name (e.g. "Create", "AimPrimary", "Walk").
    pub name: String,
    /// Raw 32-bit bytecode words for this function.
    pub code: Vec<u32>,
}

/// Parsed COB script file.
#[derive(Debug, Clone)]
pub struct CobScript {
    /// Piece names in order (index = piece number used in bytecodes).
    pub pieces: Vec<String>,
    /// Script functions with their bytecode.
    pub scripts: Vec<CobFunction>,
    /// Number of static (global) variables used by the script.
    pub num_static_vars: usize,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors specific to COB parsing.
#[derive(Debug, thiserror::Error)]
pub enum CobError {
    #[error("COB file too small: expected at least {expected} bytes, got {actual}")]
    FileTooSmall { expected: usize, actual: usize },
    #[error("invalid COB version signature: expected 4, got {0}")]
    BadVersion(i32),
    #[error("offset {offset} out of bounds (file size {file_len})")]
    OffsetOutOfBounds { offset: usize, file_len: usize },
    #[error("unterminated C string at offset {0}")]
    UnterminatedString(usize),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_i32_le(data: &[u8], offset: usize) -> Result<i32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .context("unexpected end of file reading i32")?
        .try_into()
        .unwrap();
    Ok(i32::from_le_bytes(bytes))
}

fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .context("unexpected end of file reading u32")?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(bytes))
}

/// Read a null-terminated C string starting at `offset`.
fn read_cstr(data: &[u8], offset: usize) -> Result<String> {
    ensure!(
        offset < data.len(),
        CobError::OffsetOutOfBounds {
            offset,
            file_len: data.len()
        }
    );
    let slice = &data[offset..];
    let nul_pos = slice
        .iter()
        .position(|&b| b == 0)
        .ok_or(CobError::UnterminatedString(offset))?;
    Ok(String::from_utf8_lossy(&slice[..nul_pos]).into_owned())
}

// ---------------------------------------------------------------------------
// COB header
// ---------------------------------------------------------------------------

const COB_HEADER_SIZE: usize = 28;

struct CobHeader {
    #[allow(dead_code)]
    version_sig: i32,
    num_scripts: i32,
    num_pieces: i32,
    code_length: i32, // in 32-bit words
    num_static_vars: i32,
    offset_script_name_offsets: i32,
    offset_script_code_offsets: i32,
}

fn parse_cob_header(data: &[u8]) -> Result<CobHeader> {
    ensure!(
        data.len() >= COB_HEADER_SIZE,
        CobError::FileTooSmall {
            expected: COB_HEADER_SIZE,
            actual: data.len()
        }
    );

    let version_sig = read_i32_le(data, 0)?;
    ensure!(version_sig == 4, CobError::BadVersion(version_sig));

    Ok(CobHeader {
        version_sig,
        num_scripts: read_i32_le(data, 4)?,
        num_pieces: read_i32_le(data, 8)?,
        code_length: read_i32_le(data, 12)?,
        num_static_vars: read_i32_le(data, 16)?,
        offset_script_name_offsets: read_i32_le(data, 20)?,
        offset_script_code_offsets: read_i32_le(data, 24)?,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a COB (compiled BOS) file from an in-memory byte slice.
///
/// Returns the parsed script with piece names, script functions, and
/// bytecode ready for execution by a COB VM.
pub fn parse_cob(data: &[u8]) -> Result<CobScript> {
    let header = parse_cob_header(data)?;

    // --- Piece names ---
    // Piece name offsets are stored right after the header (28 bytes).
    let piece_name_offsets_start = COB_HEADER_SIZE;
    let mut pieces = Vec::with_capacity(header.num_pieces as usize);
    for i in 0..header.num_pieces as usize {
        let name_offset = read_i32_le(data, piece_name_offsets_start + i * 4)? as usize;
        let name = read_cstr(data, name_offset)?;
        pieces.push(name);
    }

    // --- Script names ---
    let script_name_offsets_base = header.offset_script_name_offsets as usize;
    let mut script_names = Vec::with_capacity(header.num_scripts as usize);
    for i in 0..header.num_scripts as usize {
        let name_offset = read_i32_le(data, script_name_offsets_base + i * 4)? as usize;
        let name = read_cstr(data, name_offset)?;
        script_names.push(name);
    }

    // --- Script code offsets ---
    let script_code_offsets_base = header.offset_script_code_offsets as usize;
    let mut code_offsets = Vec::with_capacity(header.num_scripts as usize);
    for i in 0..header.num_scripts as usize {
        let off = read_i32_le(data, script_code_offsets_base + i * 4)? as usize;
        code_offsets.push(off);
    }

    // Build script functions by extracting bytecode for each script.
    // Code offsets are absolute word indices (byte_offset = word_index * 4).
    // Each script runs from its code_offset until the next script's offset
    // (or end of the code region).

    // Compute the end of the code region using the smallest offset + code_length.
    let code_region_end = if !code_offsets.is_empty() {
        let min_offset = *code_offsets.iter().min().unwrap();
        min_offset + header.code_length as usize
    } else {
        data.len() / 4
    };

    let mut scripts = Vec::with_capacity(header.num_scripts as usize);
    for i in 0..header.num_scripts as usize {
        let start_word = code_offsets[i];
        // End is the nearest code offset after this one, or end of code region.
        let mut end_word = code_region_end;
        for &off in &code_offsets {
            if off > start_word && off < end_word {
                end_word = off;
            }
        }

        let mut code = Vec::with_capacity(end_word.saturating_sub(start_word));
        for w in start_word..end_word {
            let byte_offset = w * 4;
            if byte_offset + 4 <= data.len() {
                code.push(read_u32_le(data, byte_offset)?);
            }
        }

        scripts.push(CobFunction {
            name: script_names[i].clone(),
            code,
        });
    }

    Ok(CobScript {
        pieces,
        scripts,
        num_static_vars: header.num_static_vars as usize,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid COB binary for testing.
    ///
    /// Layout:
    ///   [0..28)   header
    ///   [28..28+num_pieces*4)  piece name offset table
    ///   [after piece offsets]  script name offset table
    ///   [after script name offsets]  script code offset table
    ///   [string area]  null-terminated piece/script names
    ///   [code area]    bytecode words
    fn build_test_cob(
        piece_names: &[&str],
        script_names: &[&str],
        script_codes: &[Vec<u32>],
    ) -> Vec<u8> {
        assert_eq!(script_names.len(), script_codes.len());

        let num_pieces = piece_names.len();
        let num_scripts = script_names.len();

        // We'll lay out:
        //   header: 28 bytes
        //   piece name offsets: num_pieces * 4
        //   script name offsets: num_scripts * 4 (pointed to by header)
        //   script code offsets: num_scripts * 4 (pointed to by header)
        //   string data: piece names + script names (null-terminated)
        //   code data: all script bytecode words

        let piece_offsets_start = COB_HEADER_SIZE;
        let script_name_offsets_start = piece_offsets_start + num_pieces * 4;
        let script_code_offsets_start = script_name_offsets_start + num_scripts * 4;
        let strings_start = script_code_offsets_start + num_scripts * 4;

        // Compute string offsets.
        let mut string_offsets = Vec::new();
        let mut cursor = strings_start;
        for &name in piece_names.iter().chain(script_names.iter()) {
            string_offsets.push(cursor);
            cursor += name.len() + 1; // +1 for null terminator
        }

        // Code area starts after strings, aligned to 4 bytes.
        let code_start = (cursor + 3) & !3;

        // Compute code word offsets (as absolute word indices from file start).
        let mut code_word_offsets = Vec::new();
        let mut word_cursor = code_start / 4;
        for code in script_codes {
            code_word_offsets.push(word_cursor);
            word_cursor += code.len();
        }
        let total_code_words = word_cursor - (code_start / 4);

        // Build the binary.
        let total_size = code_start + total_code_words * 4;
        let mut buf = vec![0u8; total_size];

        // Header.
        buf[0..4].copy_from_slice(&4i32.to_le_bytes()); // version_sig
        buf[4..8].copy_from_slice(&(num_scripts as i32).to_le_bytes());
        buf[8..12].copy_from_slice(&(num_pieces as i32).to_le_bytes());
        buf[12..16].copy_from_slice(&(total_code_words as i32).to_le_bytes());
        buf[16..20].copy_from_slice(&0i32.to_le_bytes()); // num_static_vars
        buf[20..24].copy_from_slice(&(script_name_offsets_start as i32).to_le_bytes());
        buf[24..28].copy_from_slice(&(script_code_offsets_start as i32).to_le_bytes());

        // Piece name offsets.
        for (i, &off) in string_offsets[..num_pieces].iter().enumerate() {
            let pos = piece_offsets_start + i * 4;
            buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
        }

        // Script name offsets.
        for (i, &off) in string_offsets[num_pieces..].iter().enumerate() {
            let pos = script_name_offsets_start + i * 4;
            buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
        }

        // Script code offsets (word indices).
        for (i, &off) in code_word_offsets.iter().enumerate() {
            let pos = script_code_offsets_start + i * 4;
            buf[pos..pos + 4].copy_from_slice(&(off as i32).to_le_bytes());
        }

        // String data.
        let mut str_cursor = strings_start;
        for &name in piece_names.iter().chain(script_names.iter()) {
            buf[str_cursor..str_cursor + name.len()].copy_from_slice(name.as_bytes());
            buf[str_cursor + name.len()] = 0; // null terminator
            str_cursor += name.len() + 1;
        }

        // Code data.
        let mut code_byte_cursor = code_start;
        for code in script_codes {
            for &word in code {
                buf[code_byte_cursor..code_byte_cursor + 4].copy_from_slice(&word.to_le_bytes());
                code_byte_cursor += 4;
            }
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
        let mut data = vec![0u8; 28];
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
}
