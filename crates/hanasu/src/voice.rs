//! Loading a Kokoro voice from a raw style file.
//!
//! The `onnx-community/Kokoro-82M-v1.0-ONNX` repo ships one file per voice under
//! `voices/<name>.bin`: a bare little-endian f32 array of shape `(rows, 256)` —
//! one `[256]` style vector per phoneme-token count — with no npy/zip framing.

use crate::{Error, Result};
use std::path::Path;

const STYLE_DIM: usize = 256;
const ROW_BYTES: usize = STYLE_DIM * 4; // one [256] f32 row

/// A voice's style vectors. Synthesis picks the row matching the token count of
/// the utterance (see [`Voice::style`]).
pub(crate) struct Voice {
    styles: Vec<f32>,
    rows: usize,
}

impl Voice {
    /// Load voice `name` from `<voices_dir>/<name>.bin`.
    pub(crate) fn load(voices_dir: &Path, name: &str) -> Result<Self> {
        let path = voices_dir.join(format!("{name}.bin"));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::VoiceNotFound(name.to_string()));
            }
            Err(e) => return Err(e.into()),
        };
        Self::from_raw(&bytes)
    }

    /// Style vector for a token count, clamped to the last available row.
    pub(crate) fn style(&self, token_count: usize) -> &[f32] {
        let row = token_count.min(self.rows.saturating_sub(1));
        &self.styles[row * STYLE_DIM..(row + 1) * STYLE_DIM]
    }

    /// Parse a raw little-endian f32 buffer that must be a whole number of
    /// `[STYLE_DIM]` rows.
    fn from_raw(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(ROW_BYTES) {
            return Err(Error::VoicesParse(format!(
                "{} bytes is not a whole number of {STYLE_DIM}-float rows",
                bytes.len()
            )));
        }
        let styles: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Ok(Self {
            rows: styles.len() / STYLE_DIM,
            styles,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw voice buffer: `rows` rows of `[STYLE_DIM]`, row `r` filled with `fill(r)`.
    fn raw(rows: usize, fill: impl Fn(usize) -> f32) -> Vec<u8> {
        let mut out = Vec::with_capacity(rows * ROW_BYTES);
        for r in 0..rows {
            for _ in 0..STYLE_DIM {
                out.extend_from_slice(&fill(r).to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn parses_rows_and_selects_by_count() {
        let v = Voice::from_raw(&raw(3, |r| r as f32 / 10.0)).unwrap();
        assert_eq!(v.rows, 3);
        assert!(v.style(0).iter().all(|&x| x == 0.0));
        assert!(v.style(1).iter().all(|&x| x == 0.1));
        assert_eq!(v.style(2).len(), STYLE_DIM);
    }

    #[test]
    fn clamps_count_past_last_row() {
        let v = Voice::from_raw(&raw(2, |r| r as f32)).unwrap();
        assert!(v.style(999).iter().all(|&x| x == 1.0));
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(Voice::from_raw(&[]), Err(Error::VoicesParse(_))));
    }

    #[test]
    fn rejects_misaligned_data() {
        let mut bytes = raw(1, |_| 0.0);
        bytes.truncate(bytes.len() - 3);
        assert!(matches!(
            Voice::from_raw(&bytes),
            Err(Error::VoicesParse(_))
        ));
    }

    #[test]
    fn load_reads_named_file_and_reports_missing() {
        let dir = std::env::temp_dir().join("hanasu-voice-load");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("af_test.bin"), raw(4, |r| r as f32)).unwrap();

        let v = Voice::load(&dir, "af_test").unwrap();
        assert_eq!(v.rows, 4);
        assert!(matches!(
            Voice::load(&dir, "nope"),
            Err(Error::VoiceNotFound(_))
        ));
    }
}
