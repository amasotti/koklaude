//! The engine: load the Kokoro model + one voice once, then `synth`.
//!
//! Skeleton (slice 1) — public API surface only; the pipeline is wired in the
//! following slices (voice loading → g2p → tokenizer → inference).

use std::path::Path;

use crate::Result;

/// Synthesized speech: mono PCM samples and their sample rate.
#[derive(Debug, Clone)]
pub struct Audio {
    /// f32 samples in roughly `[-1.0, 1.0]`.
    pub samples: Vec<f32>,
    /// Samples per second (Kokoro: 24 kHz).
    pub sample_rate: u32,
}

/// Loads the Kokoro model and a single voice once, then turns text into [`Audio`].
///
/// The engine is pure DSP: it returns samples, with no opinion on WAV encoding or
/// playback (the binary owns that).
pub struct Engine {
    // Fields (ONNX session, style vector, speed) are added in later slices.
}

impl Engine {
    /// Load the Kokoro model and a voice.
    ///
    /// - `model_path` — the `kokoro-v1.0.onnx` weights.
    /// - `voices_path` — the voices npz (`voices-v1.0.bin`).
    /// - `voice` — which voice to use, e.g. `"af_heart"`.
    /// - `speed` — pace multiplier (`1.0` = normal).
    pub fn load(
        _model_path: &Path,
        _voices_path: &Path,
        _voice: &str,
        _speed: f32,
    ) -> Result<Self> {
        todo!("slices 2 + 5: load voice from npz, then the ONNX session")
    }

    /// Synthesize speech for `text`.
    pub fn synth(&self, _text: &str) -> Result<Audio> {
        todo!("slices 3-5: g2p -> tokenize -> inference -> samples")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_carries_samples_and_rate() {
        let audio = Audio {
            samples: vec![0.0, 0.5, -0.5],
            sample_rate: crate::SAMPLE_RATE,
        };
        assert_eq!(audio.samples.len(), 3);
        assert_eq!(audio.sample_rate, 24_000);
    }
}
