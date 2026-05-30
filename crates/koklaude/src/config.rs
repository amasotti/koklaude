//! Runtime configuration: where the model/voices live and how to speak.
//!
//! Voice + speed come from `~/.config/koklaude/config.toml` if present, else
//! built-in defaults. Phase 5 `init` *writes* that file (install params); here
//! we only *read* it. CLI flags (`say --voice/--speed`) override per-call.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Env var to override the koklaude home (used by tests).
const HOME_ENV: &str = "KOKLAUDE_HOME";

const MODEL_FILE: &str = "kokoro-v1.0.onnx";
const VOICES_FILE: &str = "voices-v1.0.bin";
const CONFIG_FILE: &str = "config.toml";

/// Provisional default voice — plan's "best default voice" is still open.
const DEFAULT_VOICE: &str = "af_heart";
const DEFAULT_SPEED: f32 = 1.0;

/// Resolved runtime config.
pub struct Config {
    pub home: PathBuf,
    pub voice: String,
    pub speed: f32,
}

/// The on-disk config file. Every field optional → omitted = default.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    voice: Option<String>,
    speed: Option<f32>,
}

impl Config {
    /// Load config: home from env/default, voice + speed from `config.toml`
    /// (falling back to built-in defaults). Errors if the file is malformed.
    pub fn load() -> Result<Self> {
        let home = default_home();
        let file = ConfigFile::read(&home)?;
        Ok(Self {
            voice: file.voice.unwrap_or_else(|| DEFAULT_VOICE.to_string()),
            speed: file.speed.unwrap_or(DEFAULT_SPEED),
            home,
        })
    }

    pub fn model_path(&self) -> PathBuf {
        self.home.join(MODEL_FILE)
    }

    pub fn voices_path(&self) -> PathBuf {
        self.home.join(VOICES_FILE)
    }
}

impl ConfigFile {
    /// Read `<home>/config.toml`. Missing file → defaults; present-but-bad →
    /// error (a typo'd setting must be loud, not silently ignored).
    fn read(home: &Path) -> Result<Self> {
        let path = home.join(CONFIG_FILE);
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parse {path:?}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("read {path:?}")),
        }
    }
}

/// The koklaude home directory (no config-file parsing — just the path).
pub fn home() -> PathBuf {
    default_home()
}

/// `$KOKLAUDE_HOME` if set, else `~/.config/koklaude`.
fn default_home() -> PathBuf {
    if let Ok(dir) = std::env::var(HOME_ENV) {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".config/koklaude")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test scratch dir under temp (no env mutation → no test races).
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-cfg-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = scratch("missing");
        let _ = std::fs::remove_file(dir.join(CONFIG_FILE));
        let file = ConfigFile::read(&dir).unwrap();
        assert!(file.voice.is_none());
        assert!(file.speed.is_none());
    }

    #[test]
    fn parses_voice_and_speed() {
        let dir = scratch("full");
        std::fs::write(dir.join(CONFIG_FILE), "voice = \"am_adam\"\nspeed = 1.3\n").unwrap();
        let file = ConfigFile::read(&dir).unwrap();
        assert_eq!(file.voice.as_deref(), Some("am_adam"));
        assert_eq!(file.speed, Some(1.3));
    }

    #[test]
    fn partial_file_leaves_other_default() {
        let dir = scratch("partial");
        std::fs::write(dir.join(CONFIG_FILE), "speed = 0.8\n").unwrap();
        let file = ConfigFile::read(&dir).unwrap();
        assert!(file.voice.is_none());
        assert_eq!(file.speed, Some(0.8));
    }

    #[test]
    fn malformed_file_errors() {
        let dir = scratch("bad");
        std::fs::write(dir.join(CONFIG_FILE), "this is = not valid = toml").unwrap();
        assert!(ConfigFile::read(&dir).is_err());
    }

    #[test]
    fn unknown_key_errors() {
        let dir = scratch("unknown");
        std::fs::write(dir.join(CONFIG_FILE), "voce = \"typo\"\n").unwrap();
        assert!(ConfigFile::read(&dir).is_err());
    }
}
