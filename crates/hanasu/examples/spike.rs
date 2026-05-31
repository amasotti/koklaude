//! Phase 1 engine spike — **THROWAWAY**.
//!
//! De-risks the one genuine unknown before any real engine code: Kokoro-82M's
//! ONNX I/O contract (tensor names/shapes, and that our phoneme→id mapping is
//! right).
//!
//! VERIFIED CONTRACT (printed by this spike against kokoro-v1.0.onnx):
//!   inputs : `tokens` int64 [1, seq]   ← NOT `input_ids` (the common assumption)
//!            `style`  f32   [1, 256]
//!            `speed`  f32   [1]
//!   output : `audio`  f32   [audio_length]   (mono PCM @ 24 kHz)
//!
//! It now shows the **whole chain** so you can watch g2p happen:
//!   text → [espeak-ng] → IPA phonemes → [Kokoro vocab] → token ids → [ONNX] → audio
//!
//! NOTE: the tokenizer here is the *naive* char→id map — good enough to see the
//! flow, NOT the final tokenizer. Real Misaki normalization (stress handling,
//! multi-char phonemes) is built with unit tests in Phase 2. g2p is fixed to
//! `en-us`; non-English voices still get English phonemes (a timbre swap, not
//! correct foreign pronunciation).
//!
//! Prereqs (see docs/spike.md):
//!   * `espeak-ng` on PATH        (macOS: `brew install espeak-ng`)
//!   * `unzip` on PATH            (ships with macOS/most Linux)
//!   * ~/.config/koklaude/kokoro-v1.0.onnx
//!   * ~/.config/koklaude/voices-v1.0.bin
//!
//! Run (defaults: voice `af_heart`, text "Hello world"):
//!   cargo run -p hanasu --example spike
//!   cargo run -p hanasu --example spike -- bm_george "Good evening"
//! Listen: afplay /tmp/koklaude-spike.wav

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

/// Kokoro outputs mono PCM at 24 kHz.
const SAMPLE_RATE: u32 = 24_000;

const OUT_WAV: &str = "/tmp/koklaude-spike.wav";

/// The canonical Kokoro phoneme vocabulary (hexgrad/Kokoro-82M `config.json` ›
/// `vocab`). Each IPA char maps to a token id; `0` is the boundary/pad token.
#[rustfmt::skip]
const VOCAB: &[(char, i64)] = &[
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

/// Known-good ids for "Hello world" (`həlˈoʊ wˈɜːld`), *before* boundary 0s.
/// The live g2p below must reproduce exactly these — our regression anchor.
const KNOWN_HELLO_WORLD: &[i64] = &[50, 83, 54, 156, 57, 135, 16, 65, 156, 87, 158, 54, 46];

fn koklaude_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".config/koklaude")
}

/// Link A: text → IPA phonemes, via the `espeak-ng` CLI (g2p fixed to en-us).
fn phonemize(text: &str) -> Result<String, Box<dyn Error>> {
    let out = Command::new("espeak-ng")
        .args(["-q", "--ipa", "-v", "en-us", text])
        .output()
        .map_err(|e| {
            format!("espeak-ng not runnable ({e}); install with `brew install espeak-ng`")
        })?;
    if !out.status.success() {
        return Err(format!("espeak-ng failed: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

/// Link B: IPA phonemes → token ids (naive char map). Returns the ids and any
/// chars not in the vocab (skipped) — so curiosity inputs never crash the spike.
fn tokenize(phonemes: &str) -> (Vec<i64>, Vec<char>) {
    let mut ids = Vec::new();
    let mut unknown = Vec::new();
    for ch in phonemes.chars() {
        match VOCAB.iter().find(|(c, _)| *c == ch) {
            Some((_, id)) => ids.push(*id),
            None => unknown.push(ch),
        }
    }
    (ids, unknown)
}

/// Read one 256-dim style vector (row `row`) for `voice`, straight from the
/// voices npz via `unzip -p` (no pre-extraction, works for any of the 54 voices).
/// Each entry is a numpy v1 `.npy`, dtype `<f4`, shape `(510, 1, 256)`.
fn load_style(npz: &Path, voice: &str, row: usize) -> Result<Vec<f32>, Box<dyn Error>> {
    let entry = format!("{voice}.npy");
    let out = Command::new("unzip")
        .args(["-p", npz.to_str().ok_or("non-utf8 npz path")?, &entry])
        .output()
        .map_err(|e| format!("unzip not runnable: {e}"))?;
    let bytes = out.stdout;
    if !out.status.success() || bytes.len() < 10 {
        return Err(format!(
            "voice '{voice}' not found in {npz:?} — list voices with `unzip -l {npz:?}`"
        )
        .into());
    }

    assert_eq!(&bytes[0..6], b"\x93NUMPY", "voice entry is not a .npy file");
    let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let data_off = 10 + header_len;

    // (510, 1, 256) flattens to 510 contiguous rows of 256 f32.
    let start = data_off + row * 256 * 4;
    let mut style = Vec::with_capacity(256);
    for i in 0..256 {
        let o = start + i * 4;
        style.push(f32::from_le_bytes([
            bytes[o],
            bytes[o + 1],
            bytes[o + 2],
            bytes[o + 3],
        ]));
    }
    Ok(style)
}

fn main() -> Result<(), Box<dyn Error>> {
    // args: [voice] [text...]  — defaults below.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let voice = args
        .first()
        .cloned()
        .unwrap_or_else(|| "af_heart".to_string());
    let text = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        "Hello world".to_string()
    };

    let dir = koklaude_dir();
    let model_path = dir.join("kokoro-v1.0.onnx");
    let npz_path = dir.join("voices-v1.0.bin");

    println!("voice: {voice}");
    println!("text:  {text:?}");

    // ── The chain, link by link ────────────────────────────────────────────
    let phonemes = phonemize(&text)?;
    println!("\n[g2p]   espeak-ng en-us → {phonemes:?}");

    let (ids, unknown) = tokenize(&phonemes);
    println!("[token] {} ids: {ids:?}", ids.len());
    if !unknown.is_empty() {
        println!(
            "[token] WARNING: {} char(s) not in vocab, skipped: {unknown:?}",
            unknown.len()
        );
    }
    if text == "Hello world" {
        let ok = ids == KNOWN_HELLO_WORLD;
        println!(
            "[check] live g2p vs known-good [50,83,…]: {}",
            if ok { "✓ match" } else { "✗ MISMATCH" }
        );
        assert!(ok, "live g2p diverged from the verified known-good ids");
    }

    // Boundary tokens Kokoro expects: a leading + trailing 0 around the ids.
    let mut tokens = Vec::with_capacity(ids.len() + 2);
    tokens.push(0);
    tokens.extend_from_slice(&ids);
    tokens.push(0);

    // ── Inference ──────────────────────────────────────────────────────────
    println!("\nmodel: {model_path:?}");
    let mut session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(&model_path)?;

    println!("=== ONNX inputs ===");
    for i in session.inputs() {
        println!("  {:<12} {:?}", i.name(), i.dtype());
    }
    println!("=== ONNX outputs ===");
    for o in session.outputs() {
        println!("  {:<12} {:?}", o.name(), o.dtype());
    }

    // Style is selected by the token count *before* the boundary 0s.
    let style = load_style(&npz_path, &voice, ids.len())?;
    let n = tokens.len();

    // Tensor names verified empirically: `tokens` (not `input_ids`), `style`,
    // `speed`; output is `audio`. See the printout above.
    let tokens_t = Tensor::from_array(([1_i64, n as i64], tokens))?;
    let style_t = Tensor::from_array(([1_i64, 256_i64], style))?;
    let speed_t = Tensor::from_array(([1_i64], vec![1.0_f32]))?;

    let out_name = session.outputs()[0].name().to_owned();

    println!("\nrunning inference ({n} tokens)…");
    let outputs = session.run(ort::inputs![
        "tokens" => tokens_t,
        "style"  => style_t,
        "speed"  => speed_t,
    ])?;

    let (shape, samples) = outputs[out_name.as_str()].try_extract_tensor::<f32>()?;
    println!(
        "output '{out_name}' shape={shape:?}, {} samples",
        samples.len()
    );

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(OUT_WAV, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;

    let secs = samples.len() as f32 / SAMPLE_RATE as f32;
    println!("\nwrote {OUT_WAV}  ({secs:.2}s)");
    println!("listen:  afplay {OUT_WAV}");
    Ok(())
}
