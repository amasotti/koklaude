//! Phase 1 engine spike — **THROWAWAY**.
//!
//! De-risks the one genuine unknown before any real engine code: Kokoro-82M's
//! ONNX I/O contract (tensor names/shapes, and that our phoneme→id mapping is
//! right). See `docs/plan.md` › Phase 1.
//!
//! VERIFIED CONTRACT (printed by this spike against kokoro-v1.0.onnx):
//!   inputs : `tokens` int64 [1, seq]   ← NOT `input_ids` (the common assumption)
//!            `style`  f32   [1, 256]
//!            `speed`  f32   [1]
//!   output : `audio`  f32   [audio_length]   (mono PCM @ 24 kHz)
//!
//! Deliberately minimal: it uses **neither** espeak **nor** a tokenizer (both
//! are Phase 2). Instead it feeds a *verified, hardcoded* token sequence for the
//! phrase "Hello world" plus the `af_heart` voice, runs one inference, prints
//! the model's real input/output contract, and writes a WAV to listen to.
//!
//! How the constants below were produced (all reproducible):
//!   1. `espeak-ng -q --ipa -v en-us "Hello world"`  →  `həlˈoʊ wˈɜːld`
//!   2. each char mapped through the canonical Kokoro vocab
//!      (hexgrad/Kokoro-82M `config.json` › `vocab`)  →  the ids below
//!   3. wrapped with a leading/trailing `0` (the boundary tokens Kokoro expects)
//!
//! Prereqs (downloaded once, outside the repo — see plan):
//!   ~/.claude/koklaude/kokoro-v1.0.onnx
//!   ~/.claude/koklaude/af_heart.npy   (one voice, extracted from voices-v1.0.bin)
//!
//! Run:    cargo run -p hanasu --example spike
//! Listen: afplay /tmp/koklaude-spike.wav

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

/// Kokoro outputs mono PCM at 24 kHz.
const SAMPLE_RATE: u32 = 24_000;

/// "Hello world" → `həlˈoʊ wˈɜːld` → Kokoro vocab ids, wrapped with boundary 0s.
const INPUT_IDS: [i64; 15] = [0, 50, 83, 54, 156, 57, 135, 16, 65, 156, 87, 158, 54, 46, 0];

/// Style row to use: the number of phoneme tokens *before* the boundary 0s (13).
/// Kokoro selects the style vector by token count.
const STYLE_ROW: usize = 13;

const OUT_WAV: &str = "/tmp/koklaude-spike.wav";

fn koklaude_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".claude/koklaude")
}

/// Read one 256-dim style vector (row `row`) from an `af_*.npy` file.
/// Layout: numpy v1 `.npy`, dtype `<f4`, shape `(510, 1, 256)`.
fn load_style(path: &PathBuf, row: usize) -> Result<Vec<f32>, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    // npy header: 6-byte magic, 2-byte version, 2-byte little-endian header length.
    assert_eq!(&bytes[0..6], b"\x93NUMPY", "{path:?} is not a .npy file");
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
    let dir = koklaude_dir();
    let model_path = dir.join("kokoro-v1.0.onnx");
    let voice_path = dir.join("af_heart.npy");

    println!("model: {model_path:?}");
    let mut session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(&model_path)?;

    // The de-risk itself: print the real contract straight from the model.
    println!("\n=== ONNX inputs ===");
    for i in session.inputs() {
        println!("  {:<12} {:?}", i.name(), i.dtype());
    }
    println!("=== ONNX outputs ===");
    for o in session.outputs() {
        println!("  {:<12} {:?}", o.name(), o.dtype());
    }

    let style = load_style(&voice_path, STYLE_ROW)?;
    let n = INPUT_IDS.len();

    // Tensor names verified empirically against the model: `tokens` (not
    // `input_ids`), `style`, `speed`; output is `audio`. See the printout above.
    let tokens = Tensor::from_array(([1_i64, n as i64], INPUT_IDS.to_vec()))?;
    let style_t = Tensor::from_array(([1_i64, 256_i64], style))?;
    let speed_t = Tensor::from_array(([1_i64], vec![1.0_f32]))?;

    let out_name = session.outputs()[0].name().to_owned();

    println!("\nrunning inference ({n} tokens)…");
    let outputs = session.run(ort::inputs![
        "tokens" => tokens,
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
