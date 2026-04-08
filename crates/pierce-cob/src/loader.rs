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

/// Spring COB header: 44 bytes (11 i32 fields).
const COB_HEADER_SIZE: usize = 44;

#[allow(dead_code)]
struct CobHeader {
    version_sig: i32,
    num_scripts: i32,
    num_pieces: i32,
    code_length: i32, // in 32-bit words
    num_static_vars: i32,
    offset_script_code_offsets: i32,
    offset_script_name_offsets: i32,
    offset_piece_name_offsets: i32,
    offset_code_start: i32,
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
        // offset 20 is unused (always 0)
        offset_script_code_offsets: read_i32_le(data, 24)?,
        offset_script_name_offsets: read_i32_le(data, 28)?,
        offset_piece_name_offsets: read_i32_le(data, 32)?,
        offset_code_start: read_i32_le(data, 36)?,
        // offset 40 is unused (always 0)
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
    let piece_name_offsets_start = header.offset_piece_name_offsets as usize;
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
    // Offsets are word indices relative to code_start.
    let script_code_offsets_base = header.offset_script_code_offsets as usize;
    let code_start = header.offset_code_start as usize;
    let mut code_offsets = Vec::with_capacity(header.num_scripts as usize);
    for i in 0..header.num_scripts as usize {
        let off = read_i32_le(data, script_code_offsets_base + i * 4)? as usize;
        code_offsets.push(off);
    }

    // Build script functions by extracting bytecode for each script.
    // code_offsets[i] is a word index relative to code_start.
    // Absolute byte offset = code_start + word_index * 4.
    let code_length_words = header.code_length as usize;

    let mut scripts = Vec::with_capacity(header.num_scripts as usize);
    for i in 0..header.num_scripts as usize {
        let start_word = code_offsets[i];
        // End is the nearest code offset after this one, or end of code region.
        let mut end_word = code_length_words;
        for &off in &code_offsets {
            if off > start_word && off < end_word {
                end_word = off;
            }
        }

        let mut code = Vec::with_capacity(end_word.saturating_sub(start_word));
        for w in start_word..end_word {
            let byte_offset = code_start + w * 4;
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
#[path = "tests/loader_tests.rs"]
mod tests;
