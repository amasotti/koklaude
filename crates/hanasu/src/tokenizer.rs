//! Text → Kokoro phoneme-token ids.
//!
//! espeak drops punctuation, but Kokoro uses punctuation tokens for pauses, so we
//! split the text on those marks ourselves, phonemize each punctuation-free chunk,
//! re-insert the marks, then map the result through the vocab. Matches the
//! kokoro-onnx reference (`preserve_punctuation=True`). See docs/phase2-engine-api.md.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::{MAX_PHONEME_LENGTH, Result, g2p};

/// Canonical Kokoro phoneme vocabulary (hexgrad/Kokoro-82M `config.json` › `vocab`).
#[rustfmt::skip]
const IPA_VOCABULARY: &[(char, i64)] = &[
    (';', 1), (':', 2), (',', 3), ('.', 4), ('!', 5), ('?', 6),
    ('—', 9), ('…', 10), ('"', 11), ('(', 12), (')', 13), ('“', 14),
    ('”', 15), (' ', 16), ('̃', 17), ('ʣ', 18), ('ʥ', 19), ('ʦ', 20),
    ('ʨ', 21), ('ᵝ', 22), ('ꭧ', 23), ('A', 24), ('I', 25), ('O', 31),
    ('Q', 33), ('S', 35), ('T', 36), ('W', 39), ('Y', 41), ('ᵊ', 42),
    ('a', 43), ('b', 44), ('c', 45), ('d', 46), ('e', 47), ('f', 48),
    ('h', 50), ('i', 51), ('j', 52), ('k', 53), ('l', 54), ('m', 55),
    ('n', 56), ('o', 57), ('p', 58), ('q', 59), ('r', 60), ('s', 61),
    ('t', 62), ('u', 63), ('v', 64), ('w', 65), ('x', 66), ('y', 67),
    ('z', 68), ('ɑ', 69), ('ɐ', 70), ('ɒ', 71), ('æ', 72), ('β', 75),
    ('ɔ', 76), ('ɕ', 77), ('ç', 78), ('ɖ', 80), ('ð', 81), ('ʤ', 82),
    ('ə', 83), ('ɚ', 85), ('ɛ', 86), ('ɜ', 87), ('ɟ', 90), ('ɡ', 92),
    ('ɥ', 99), ('ɨ', 101), ('ɪ', 102), ('ʝ', 103), ('ɯ', 110), ('ɰ', 111),
    ('ŋ', 112), ('ɳ', 113), ('ɲ', 114), ('ɴ', 115), ('ø', 116), ('ɸ', 118),
    ('θ', 119), ('œ', 120), ('ɹ', 123), ('ɾ', 125), ('ɻ', 126), ('ʁ', 128),
    ('ɽ', 129), ('ʂ', 130), ('ʃ', 131), ('ʈ', 132), ('ʧ', 133), ('ʊ', 135),
    ('ʋ', 136), ('ʌ', 138), ('ɣ', 139), ('ɤ', 140), ('χ', 142), ('ʎ', 143),
    ('ʒ', 147), ('ʔ', 148), ('ˈ', 156), ('ˌ', 157), ('ː', 158), ('ʰ', 162),
    ('ʲ', 164), ('↓', 169), ('→', 171), ('↗', 172), ('↘', 173), ('ᵻ', 177),
];

/// Punctuation Kokoro keeps as pause tokens (the marks present in `VOCAB`).
const PUNCTUATION: &[char] = &[
    ';', ':', ',', '.', '!', '?', '—', '…', '"', '(', ')', '“', '”',
];

/// Convert text to Kokoro token ids, ready for inference (boundary `0`s are added
/// by the caller). Clamped to [`MAX_PHONEME_LENGTH`].
pub(crate) fn tokenize(text: &str) -> Result<Vec<i64>> {
    let phonemes = phonemize_with_marks(text)?;
    Ok(encode(&phonemes))
}

#[derive(Debug, PartialEq)]
enum Segment {
    Text(String),
    Mark(char),
}

/// Split text into phonemizable chunks and the punctuation marks between them.
fn split_on_marks(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut chunk = String::new();
    for ch in text.chars() {
        if PUNCTUATION.contains(&ch) {
            if !chunk.is_empty() {
                segments.push(Segment::Text(std::mem::take(&mut chunk)));
            }
            segments.push(Segment::Mark(ch));
        } else {
            chunk.push(ch);
        }
    }
    if !chunk.is_empty() {
        segments.push(Segment::Text(chunk));
    }
    segments
}

/// Phonemize each chunk via espeak, re-inserting the marks (attached to the
/// preceding phonemes, with a space before the next chunk).
fn phonemize_with_marks(text: &str) -> Result<String> {
    let mut out = String::new();
    for segment in split_on_marks(text) {
        match segment {
            Segment::Mark(mark) => out.push(mark),
            Segment::Text(chunk) if !chunk.trim().is_empty() => {
                let phonemes = g2p::phonemize(&chunk)?;
                if !phonemes.is_empty() {
                    if !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                    out.push_str(&phonemes);
                }
            }
            Segment::Text(_) => {}
        }
    }
    Ok(out)
}

/// Map a phoneme string to vocab ids, dropping unknown chars and clamping length.
fn encode(phonemes: &str) -> Vec<i64> {
    let vocab = vocab();
    phonemes
        .chars()
        .filter_map(|c| vocab.get(&c).copied())
        .take(MAX_PHONEME_LENGTH)
        .collect()
}

fn vocab() -> &'static HashMap<char, i64> {
    static VOCAB_MAP: OnceLock<HashMap<char, i64>> = OnceLock::new();
    VOCAB_MAP.get_or_init(|| IPA_VOCABULARY.iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_maps_known_phrase() {
        // "Hello world" phonemes — the Phase 1 verified ids (before boundary 0s).
        let ids = encode("həlˈoʊ wˈɜːld");
        assert_eq!(
            ids,
            [50, 83, 54, 156, 57, 135, 16, 65, 156, 87, 158, 54, 46]
        );
    }

    #[test]
    fn encode_keeps_punctuation_tokens() {
        assert_eq!(encode("a,b."), [43, 3, 44, 4]);
    }

    #[test]
    fn encode_drops_unknown_chars() {
        assert_eq!(encode("a∑b"), [43, 44]); // ∑ is not in the vocab
    }

    #[test]
    fn encode_clamps_to_max_length() {
        let ids = encode(&"a".repeat(MAX_PHONEME_LENGTH + 50));
        assert_eq!(ids.len(), MAX_PHONEME_LENGTH);
    }

    #[test]
    fn split_separates_text_and_marks() {
        assert_eq!(
            split_on_marks("Good evening, the build"),
            vec![
                Segment::Text("Good evening".into()),
                Segment::Mark(','),
                Segment::Text(" the build".into()),
            ]
        );
    }

    #[test]
    fn split_handles_edge_marks() {
        assert_eq!(
            split_on_marks("(hi)"),
            vec![
                Segment::Mark('('),
                Segment::Text("hi".into()),
                Segment::Mark(')')
            ]
        );
        assert_eq!(split_on_marks(""), vec![]);
        assert_eq!(
            split_on_marks("no marks"),
            vec![Segment::Text("no marks".into())]
        );
    }

    fn espeak_available() -> bool {
        std::process::Command::new("espeak-ng")
            .arg("--version")
            .output()
            .is_ok()
    }

    #[test]
    fn tokenize_matches_phase1_known_good() {
        if !espeak_available() {
            eprintln!("skipping tokenize_matches_phase1_known_good: espeak-ng not installed");
            return;
        }
        let ids = tokenize("Hello world").unwrap();
        assert_eq!(
            ids,
            [50, 83, 54, 156, 57, 135, 16, 65, 156, 87, 158, 54, 46]
        );
    }

    #[test]
    fn tokenize_preserves_comma_pause() {
        if !espeak_available() {
            eprintln!("skipping tokenize_preserves_comma_pause: espeak-ng not installed");
            return;
        }
        assert!(tokenize("Hi, there").unwrap().contains(&3)); // comma token survives
    }
}
