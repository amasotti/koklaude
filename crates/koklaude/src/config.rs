//! Runtime configuration: where the model/voices live and how to speak.
//!
//! Phase 3 = paths + in-code defaults only. File persistence lands with
//! Phase 5 `init` (see docs/plan.md).

use std::path::PathBuf;

/// Env var to override the koklaude home (used by tests).
const HOME_ENV: &str = "KOKLAUDE_HOME";

const MODEL_FILE: &str = "kokoro-v1.0.onnx";
const VOICES_FILE: &str = "voices-v1.0.bin";

/// Provisional default voice — plan's "best default voice" is still open.
const DEFAULT_VOICE: &str = "af_heart";
const DEFAULT_SPEED: f32 = 1.0;

/// Resolved runtime config.
pub struct Config {
    pub home: PathBuf,
    pub voice: String,
    pub speed: f32,
}

impl Config {
    /// Load config. For now: home from env/default, voice + speed hard-coded.
    pub fn load() -> Self {
        Self {
            home: default_home(),
            voice: DEFAULT_VOICE.to_string(),
            speed: DEFAULT_SPEED,
        }
    }

    pub fn model_path(&self) -> PathBuf {
        self.home.join(MODEL_FILE)
    }

    pub fn voices_path(&self) -> PathBuf {
        self.home.join(VOICES_FILE)
    }
}

/// `$KOKLAUDE_HOME` if set, else `~/.claude/koklaude`.
fn default_home() -> PathBuf {
    if let Ok(dir) = std::env::var(HOME_ENV) {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".claude/koklaude")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_env_override_wins() {
        // SAFETY: single-threaded test; we set then read our own var.
        unsafe { std::env::set_var(HOME_ENV, "/tmp/koklaude-test") };
        let cfg = Config::load();
        assert_eq!(cfg.home, PathBuf::from("/tmp/koklaude-test"));
        assert_eq!(
            cfg.model_path(),
            PathBuf::from("/tmp/koklaude-test/kokoro-v1.0.onnx")
        );
        assert_eq!(
            cfg.voices_path(),
            PathBuf::from("/tmp/koklaude-test/voices-v1.0.bin")
        );
        unsafe { std::env::remove_var(HOME_ENV) };
    }

    #[test]
    fn defaults_are_sane() {
        let cfg = Config::load();
        assert_eq!(cfg.voice, "af_heart");
        assert_eq!(cfg.speed, 1.0);
    }
}
