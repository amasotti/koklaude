//! Loading a Kokoro voice from the voices npz.
#![allow(dead_code)] // wired into Engine in slice 5

use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::result::ZipError;
use crate::{Error, Result};

const STYLE_DIM: usize = 256;
const ROW_BYTES: usize = STYLE_DIM * 4; // one [256] f32 row

/// A voice's style vectors. Kokoro stores one `[256]` style per phoneme-token
/// count (npy shape `(rows, 1, 256)`); synthesis picks the row matching the
/// token count of the utterance.
pub(crate) struct Voice {
    styles: Vec<f32>,
    rows: usize,
}

impl Voice {
    pub(crate) fn load(voices_path: &Path, name: &str) -> Result<Self> {
        let npy = read_voice_entry(voices_path, name)?;
        Self::from_npy(&npy)
    }

    /// Style vector for a token count, clamped to the last available row.
    pub(crate) fn style(&self, token_count: usize) -> &[f32] {
        let row = token_count.min(self.rows.saturating_sub(1));
        &self.styles[row * STYLE_DIM..(row + 1) * STYLE_DIM]
    }

    fn from_npy(bytes: &[u8]) -> Result<Self> {
        let data = npy_f32_rows(bytes)?;
        let styles: Vec<f32> = data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Ok(Self {
            rows: styles.len() / STYLE_DIM,
            styles,
        })
    }
}

/// Read the raw bytes of `{name}.npy` out of the voices npz.
fn read_voice_entry(voices_path: &Path, name: &str) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(File::open(voices_path)?)
        .map_err(|e| Error::VoicesParse(e.to_string()))?;

    let mut entry = match archive.by_name(&format!("{name}.npy")) {
        Ok(entry) => entry,
        Err(ZipError::FileNotFound) => {
            return Err(Error::VoiceNotFound(name.to_string()));
        }
        Err(e) => return Err(Error::VoicesParse(e.to_string())),
    };

    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Validate a numpy v1 `.npy` of little-endian f32 and return its data region,
/// which must be a whole number of `[STYLE_DIM]` rows.
fn npy_f32_rows(bytes: &[u8]) -> Result<&[u8]> {
    // .npy v1 layout: 6-byte magic | 2-byte version | 2-byte LE header length | header | data.
    const PREAMBLE: usize = 10;
    if bytes.len() < PREAMBLE || &bytes[0..6] != b"\x93NUMPY" {
        return Err(Error::VoicesParse("not a .npy file".into()));
    }

    let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let header = bytes
        .get(PREAMBLE..PREAMBLE + header_len)
        .ok_or_else(|| Error::VoicesParse("truncated npy header".into()))?;

    if !std::str::from_utf8(header).is_ok_and(|h| h.contains("<f4")) {
        return Err(Error::VoicesParse(
            "expected little-endian f32 (<f4)".into(),
        ));
    }

    let data = &bytes[PREAMBLE + header_len..];
    if data.is_empty() || !data.len().is_multiple_of(ROW_BYTES) {
        return Err(Error::VoicesParse(format!(
            "{} data bytes is not a whole number of {STYLE_DIM}-float rows",
            data.len()
        )));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `.npy` with `rows` rows of `[STYLE_DIM]`, row `r` filled
    /// with `fill(r)`.
    fn npy(rows: usize, fill: impl Fn(usize) -> f32) -> Vec<u8> {
        let header = format!(
            "{{'descr': '<f4', 'fortran_order': False, 'shape': ({rows}, 1, {STYLE_DIM}), }}\n"
        );
        let mut out = vec![0x93];
        out.extend_from_slice(b"NUMPY");
        out.extend_from_slice(&[1, 0]);
        out.extend_from_slice(&(header.len() as u16).to_le_bytes());
        out.extend_from_slice(header.as_bytes());
        for r in 0..rows {
            for _ in 0..STYLE_DIM {
                out.extend_from_slice(&fill(r).to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn parses_rows_and_selects_by_count() {
        let v = Voice::from_npy(&npy(3, |r| r as f32 / 10.0)).unwrap();
        assert_eq!(v.rows, 3);
        assert!(v.style(0).iter().all(|&x| x == 0.0));
        assert!(v.style(1).iter().all(|&x| x == 0.1));
        assert_eq!(v.style(2).len(), STYLE_DIM);
    }

    #[test]
    fn clamps_count_past_last_row() {
        let v = Voice::from_npy(&npy(2, |r| r as f32)).unwrap();
        assert!(v.style(999).iter().all(|&x| x == 1.0));
    }

    #[test]
    fn rejects_non_npy() {
        assert!(matches!(
            Voice::from_npy(b"garbage"),
            Err(Error::VoicesParse(_))
        ));
    }

    #[test]
    fn rejects_misaligned_data() {
        let mut bytes = npy(1, |_| 0.0);
        bytes.truncate(bytes.len() - 3);
        assert!(matches!(
            Voice::from_npy(&bytes),
            Err(Error::VoicesParse(_))
        ));
    }
}
