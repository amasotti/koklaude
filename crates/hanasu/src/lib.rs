//! # hanasu (話す, "to speak")
//!
//! A small Rust text-to-speech engine that runs the Kokoro-82M model via ONNX,
//! fully on-device. It is the maintained successor to the abandoned `kokoroxide`:
//! `ort` 2.0 for inference, `espeak-ng` for phonemization. GPL-3.0 (espeak).
//!
//! `hanasu` is intentionally **assistant-agnostic** and **application-agnostic**:
//! it knows nothing about Claude Code, hooks, daemons, or sockets. It does one
//! thing — turn text into speech audio — so it can be reused or extracted into
//! its own crate/repository unchanged.
//!
//! The public API (an `Engine` that loads the model once and synthesizes audio)
//! is designed together, step by step. See `../../docs/architecture.md` for the
//! intended pipeline: text → phonemes (espeak-ng) → tokens → ort/ONNX → samples.
//!
//! Nothing is implemented yet.
