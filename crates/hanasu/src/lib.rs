//! # hanasu (話す, "to speak")
//!
//! A small Rust text-to-speech engine that runs the Kokoro-82M model via ONNX,
//! fully on-device. `ort` 2.0 for inference, `espeak-ng` (external CLI) for
//! phonemization.
//!
//! `hanasu` is intentionally **assistant-agnostic** and **application-agnostic**:
//! it knows nothing about Claude Code, hooks, daemons, or sockets. It does one
//! thing — turn text into speech samples — so it can be reused or extracted into
//! its own crate/repository unchanged.
//!

mod engine;
mod error;
mod g2p;
mod voice;

pub use engine::{Audio, Engine};
pub use error::{Error, Result};

/// Kokoro outputs mono PCM at this sample rate.
pub const SAMPLE_RATE: u32 = 24_000;

/// Maximum phoneme tokens per inference — Kokoro's style array is `[510]`.
pub const MAX_PHONEME_LENGTH: usize = 510;
