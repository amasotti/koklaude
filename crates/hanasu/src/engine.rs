//! The engine: load the Kokoro model + one voice once, then `synth`.

use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;

use crate::voice::Voice;
use crate::{Error, Result, SAMPLE_RATE, tokenizer};

const STYLE_DIM: i64 = 256;

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
/// Pure DSP: returns samples, with no opinion on WAV encoding or playback. The
/// `Session` is behind a `Mutex` (ONNX `run` needs `&mut`) so `synth` takes
/// `&self` and the engine can be shared by a warm daemon.
pub struct Engine {
    session: Mutex<Session>,
    voice: Voice,
    speed: f32,
}

impl Engine {
    /// Load the Kokoro model and a voice.
    ///
    /// - `model_path` — the `kokoro-v1.0.onnx` weights.
    /// - `voices_path` — the voices npz (`voices-v1.0.bin`).
    /// - `voice` — which voice to use, e.g. `"af_heart"`.
    /// - `speed` — pace multiplier (`1.0` = normal).
    pub fn load(model_path: &Path, voices_path: &Path, voice: &str, speed: f32) -> Result<Self> {
        let session = open_session(model_path).map_err(|e| Error::ModelLoad(e.to_string()))?;

        Ok(Self {
            session: Mutex::new(session),
            voice: Voice::load(voices_path, voice)?,
            speed,
        })
    }

    /// Synthesize speech for `text`.
    pub fn synth(&self, text: &str) -> Result<Audio> {
        let ids = tokenizer::tokenize(text)?;
        if ids.is_empty() {
            return Ok(Audio {
                samples: Vec::new(),
                sample_rate: SAMPLE_RATE,
            });
        }

        // Kokoro selects the style by token count, then expects the ids wrapped
        // with a leading/trailing 0.
        let style = self.voice.style(ids.len()).to_vec();
        let mut tokens = Vec::with_capacity(ids.len() + 2);
        tokens.push(0);
        tokens.extend_from_slice(&ids);
        tokens.push(0);
        let len = tokens.len() as i64;

        let tokens = Tensor::from_array(([1, len], tokens)).map_err(infer_err)?;
        let style = Tensor::from_array(([1, STYLE_DIM], style)).map_err(infer_err)?;
        let speed = Tensor::from_array(([1], vec![self.speed])).map_err(infer_err)?;

        // Recover from poisoning: a prior panic doesn't corrupt the session, so a
        // single bad synth shouldn't permanently brick the engine.
        let mut session = self.session.lock().unwrap_or_else(|p| p.into_inner());
        let outputs = session
            .run(ort::inputs!["tokens" => tokens, "style" => style, "speed" => speed])
            .map_err(infer_err)?;
        let (_shape, samples) = outputs["audio"]
            .try_extract_tensor::<f32>()
            .map_err(infer_err)?;

        Ok(Audio {
            samples: samples.to_vec(),
            sample_rate: SAMPLE_RATE,
        })
    }
}

fn open_session(model_path: &Path) -> std::result::Result<Session, ort::Error> {
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(model_path)
}

fn infer_err(e: ort::Error) -> Error {
    Error::Inference(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn audio_carries_samples_and_rate() {
        let audio = Audio {
            samples: vec![0.0, 0.5, -0.5],
            sample_rate: SAMPLE_RATE,
        };
        assert_eq!(audio.samples.len(), 3);
        assert_eq!(audio.sample_rate, 24_000);
    }

    /// End-to-end smoke test against the real model. Skips unless the model,
    /// voices, and espeak-ng are all present (they aren't in CI / a fresh clone).
    #[test]
    fn synth_hello_world_is_audible() {
        let dir = match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(".config/koklaude"),
            Err(_) => return,
        };
        let model = dir.join("kokoro-v1.0.onnx");
        let voices = dir.join("voices-v1.0.bin");
        let espeak = std::process::Command::new("espeak-ng")
            .arg("--version")
            .output()
            .is_ok();
        if !model.exists() || !voices.exists() || !espeak {
            eprintln!("skipping synth_hello_world_is_audible: model/voices/espeak not present");
            return;
        }

        let engine = Engine::load(&model, &voices, "af_heart", 1.0).unwrap();
        let audio = engine.synth("Hello world").unwrap();

        assert_eq!(audio.sample_rate, 24_000);
        assert!(audio.samples.len() > 10_000, "expected ~1.6s of audio");
        assert!(
            audio.samples.iter().any(|&s| s.abs() > 0.01),
            "expected non-silent audio"
        );
    }
}
