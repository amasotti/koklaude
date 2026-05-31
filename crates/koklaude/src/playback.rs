//! Turn engine samples into audible sound: temp WAV + `afplay` (macOS).
//!
//! Kokoro emits f32 IEEE-float PCM; only float WAV round-trips it (decisions /
//! plan gotcha). Other-OS playback is Phase 6.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use hanasu::Audio;
use tempfile::Builder;

/// Synthesized audio → speaker. Writes a temp WAV, then blocks on `afplay`.
pub fn play(audio: &Audio) -> anyhow::Result<()> {
    let file = Builder::new()
        .prefix("koklaude-")
        .suffix(".wav")
        .tempfile()
        .context("create temp WAV")?;
    let path = file.path().to_path_buf();
    write_wav(&path, audio)?;

    let status = Command::new("afplay")
        .arg(&path)
        .status()
        .context("failed to launch afplay (macOS only)")?;
    if !status.success() {
        bail!("afplay exited with {status}");
    }
    Ok(())
}

fn write_wav(path: &Path, audio: &Audio) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).with_context(|| format!("create WAV {path:?}"))?;
    for &s in &audio.samples {
        writer.write_sample(s).context("write sample")?;
    }
    writer.finalize().context("finalize WAV")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrips_float_samples() {
        let audio = Audio {
            samples: vec![0.0, 0.5, -0.5, 1.0, -1.0],
            sample_rate: 24_000,
        };
        let path = std::env::temp_dir().join("koklaude-test-roundtrip.wav");
        write_wav(&path, &audio).unwrap();

        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 24_000);
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
        let back: Vec<f32> = reader.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(back, audio.samples);
    }

    #[test]
    fn temp_wav_names_do_not_collide() {
        let a = Builder::new()
            .prefix("koklaude-")
            .suffix(".wav")
            .tempfile()
            .unwrap();
        let b = Builder::new()
            .prefix("koklaude-")
            .suffix(".wav")
            .tempfile()
            .unwrap();
        assert_ne!(a.path(), b.path());
    }
}
