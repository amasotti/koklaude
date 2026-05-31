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
    ('—', 9), ('…', 10), ('"', 11), ('(', 12), (')', 13), ('"', 14),
    ('"', 15), (' ', 16), ('̃', 17), ('ʣ', 18), ('ʥ', 19), ('ʦ', 20),
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
    ';', ':', ',', '.', '!', '?', '—', '…', '"', '(', ')', '"', '"',
];

/// Characters that end a sentence and become a chunk boundary.
const SENTENCE_ENDS: &[char] = &['.', '!', '?', '…', '\n'];

/// Split `text` into sentences, keeping each sentence-ending character
/// attached to its sentence. Leading/trailing whitespace stripped per sentence.
/// Whitespace-only pieces are dropped.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if SENTENCE_ENDS.contains(&ch) {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        sentences.push(tail);
    }
    sentences
}

/// Conservatively split `text` at word boundaries into pieces of at most
/// `HARD_SPLIT_WORDS` words each. Used when a single sentence exceeds
/// `MAX_PHONEME_LENGTH` on its own.
///
/// 50 words × ≈5 phonemes/word = ≈250 tokens — well under the 510 limit.
/// `encode`'s existing clamp is the final safety net for pathological input.
const HARD_SPLIT_WORDS: usize = 50;

fn hard_split(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    words
        .chunks(HARD_SPLIT_WORDS)
        .map(|chunk| chunk.join(" "))
        .collect()
}

/// Recursively split oversize text until every emitted piece is under Kokoro's
/// phoneme-token limit. Prefer word boundaries; fall back to character halves
/// for pathological single-token input.
fn split_to_fit(text: &str) -> Result<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if token_count(trimmed)? <= MAX_PHONEME_LENGTH {
        return Ok(vec![trimmed.to_string()]);
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() > 1 {
        let mid = words.len() / 2;
        let mut pieces = split_to_fit(&words[..mid].join(" "))?;
        pieces.extend(split_to_fit(&words[mid..].join(" "))?);
        return Ok(pieces);
    }

    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 1 {
        return Ok(vec![trimmed.to_string()]);
    }
    let mid = chars.len() / 2;
    let left: String = chars[..mid].iter().collect();
    let right: String = chars[mid..].iter().collect();
    let mut pieces = split_to_fit(&left)?;
    pieces.extend(split_to_fit(&right)?);
    Ok(pieces)
}

/// Split `text` into chunks where each chunk's phoneme token count is at most
/// [`MAX_PHONEME_LENGTH`]. Sentences (split at `.!?\n…`) are packed greedily;
/// a sentence that exceeds the budget alone is split at word boundaries by
/// [`hard_split`].
///
/// Returns an empty `Vec` for empty or whitespace-only input.
pub(crate) fn split_into_chunks(text: &str) -> Result<Vec<String>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }

    let sentences = split_sentences(text);

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len: usize = 0;

    for sentence in sentences {
        let token_count = token_count(&sentence)?;

        if token_count == 0 {
            continue; // punctuation-only or blank
        }

        if token_count > MAX_PHONEME_LENGTH {
            // Sentence alone exceeds budget — flush current chunk first, then
            // hard-split and emit each piece directly.
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_len = 0;
            }
            for piece in hard_split(&sentence) {
                chunks.extend(split_to_fit(&piece)?);
            }
            continue;
        }

        if current_len + token_count > MAX_PHONEME_LENGTH {
            // Adding this sentence would overflow — flush and start fresh.
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(&sentence);
        current_len += token_count;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    Ok(chunks)
}

/// Convert text to Kokoro token ids, ready for inference (boundary `0`s are added
/// by the caller). Clamped to [`MAX_PHONEME_LENGTH`].
pub(crate) fn tokenize(text: &str) -> Result<Vec<i64>> {
    let phonemes = phonemize_with_marks(text)?;
    Ok(encode_all(&phonemes)
        .into_iter()
        .take(MAX_PHONEME_LENGTH)
        .collect())
}

fn token_count(text: &str) -> Result<usize> {
    let phonemes = phonemize_with_marks(text)?;
    Ok(encode_all(&phonemes).len())
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
#[cfg(test)]
fn encode(phonemes: &str) -> Vec<i64> {
    encode_all(phonemes)
        .into_iter()
        .take(MAX_PHONEME_LENGTH)
        .collect()
}

fn encode_all(phonemes: &str) -> Vec<i64> {
    let vocab = vocab();
    phonemes
        .chars()
        .filter_map(|c| vocab.get(&c).copied())
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

    // --- split_sentences ---

    #[test]
    fn split_sentences_on_period() {
        assert_eq!(
            split_sentences("Hello world. Goodbye world."),
            vec!["Hello world.", "Goodbye world."]
        );
    }

    #[test]
    fn split_sentences_on_question_and_exclaim() {
        assert_eq!(
            split_sentences("Really? Yes! Okay."),
            vec!["Really?", "Yes!", "Okay."]
        );
    }

    #[test]
    fn split_sentences_on_newline() {
        assert_eq!(
            split_sentences("Line one\nLine two"),
            vec!["Line one", "Line two"]
        );
    }

    #[test]
    fn split_sentences_no_trailing_empty() {
        let got = split_sentences("Done.");
        assert_eq!(got, vec!["Done."]);
    }

    #[test]
    fn split_sentences_empty() {
        let got: Vec<String> = split_sentences("");
        assert!(got.is_empty());
    }

    #[test]
    fn split_sentences_preserves_comma_inside() {
        assert_eq!(
            split_sentences("Hello, world. Bye."),
            vec!["Hello, world.", "Bye."]
        );
    }

    // --- hard_split ---

    #[test]
    fn hard_split_short_sentence_unchanged() {
        let got = hard_split("Short sentence here.");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], "Short sentence here.");
    }

    #[test]
    fn hard_split_long_sentence_splits() {
        let long: String = (0..150)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let got = hard_split(&long);
        assert!(
            got.len() >= 2,
            "expected at least 2 pieces, got {}",
            got.len()
        );
        for piece in &got {
            let word_count = piece.split_whitespace().count();
            assert!(
                word_count <= 50,
                "piece has {word_count} words, expected ≤50"
            );
        }
    }

    #[test]
    fn hard_split_empty() {
        assert!(hard_split("").is_empty());
    }

    // --- split_into_chunks ---

    #[test]
    fn split_into_chunks_empty() {
        let got = split_into_chunks("").unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn split_into_chunks_whitespace_only() {
        let got = split_into_chunks("   \n  ").unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn split_into_chunks_short_text_is_single_chunk() {
        if !espeak_available() {
            eprintln!("skipping: espeak-ng not installed");
            return;
        }
        let text = "Hello. How are you? I am fine.";
        let got = split_into_chunks(text).unwrap();
        assert_eq!(got.len(), 1, "short text must be a single chunk");
    }

    #[test]
    fn split_into_chunks_each_chunk_under_limit() {
        if !espeak_available() {
            eprintln!("skipping: espeak-ng not installed");
            return;
        }
        // 40 repetitions of a short sentence → forces multiple chunks
        let sentence = "The quick brown fox jumps over the lazy dog. ";
        let text: String = sentence.repeat(40);
        let got = split_into_chunks(&text).unwrap();
        assert!(
            got.len() > 1,
            "expected multiple chunks for repeated long text"
        );
        for chunk in &got {
            let tokens = tokenize(chunk).unwrap();
            assert!(
                tokens.len() <= MAX_PHONEME_LENGTH,
                "chunk has {} tokens, expected ≤{}",
                tokens.len(),
                MAX_PHONEME_LENGTH
            );
        }
    }

    #[test]
    fn split_into_chunks_rechecks_hard_split_pieces() {
        if !espeak_available() {
            eprintln!("skipping: espeak-ng not installed");
            return;
        }
        let text = (0..260)
            .map(|i| format!("supercalifragilisticexpialidocious{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let got = split_into_chunks(&text).unwrap();
        assert!(got.len() > 1, "oversize sentence should be split");
        for chunk in &got {
            let tokens = token_count(chunk).unwrap();
            assert!(
                tokens <= MAX_PHONEME_LENGTH,
                "chunk has {tokens} tokens, expected ≤{MAX_PHONEME_LENGTH}"
            );
        }
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
