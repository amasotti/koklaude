//! Runtime configuration: where the model/voices live and how to speak.
//!
//! Voice + speed come from `~/.config/koklaude/config.toml` if present, else
//! built-in defaults. Phase 5 `init` *writes* that file (install params); here
//! we only *read* it. CLI flags (`say --voice/--speed`) override per-call.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Env var to override the koklaude home (used by tests).
const HOME_ENV: &str = "KOKLAUDE_HOME";

const MODEL_FILE: &str = "kokoro-v1.0.onnx";
const VOICES_FILE: &str = "voices-v1.0.bin";
const CONFIG_FILE: &str = "config.toml";
const SOCKET_FILE: &str = "daemon.sock";

/// Default voice — Kokoro's own reference voice (highest-graded in hexgrad's
/// voice table). Overridable via `config.toml` or `say --voice`.
const DEFAULT_VOICE: &str = "af_heart";
const DEFAULT_SPEED: f32 = 1.0;
/// Free the warm model after this long with no replies (decisions D8).
const DEFAULT_IDLE_MINUTES: u64 = 30;

/// Resolved runtime config.
pub struct Config {
    pub home: PathBuf,
    pub voice: String,
    pub speed: f32,
    pub idle_timeout: Duration,
}

/// The on-disk config file. Every field optional → omitted = default.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    voice: Option<String>,
    speed: Option<f32>,
    idle_timeout_minutes: Option<u64>,
}

impl Config {
    /// Load config: home from env/default, voice + speed from `config.toml`
    /// (falling back to built-in defaults). Errors if the file is malformed.
    pub fn load() -> Result<Self> {
        let home = default_home();
        let file = ConfigFile::read(&home)?;
        let idle_minutes = file.idle_timeout_minutes.unwrap_or(DEFAULT_IDLE_MINUTES);
        Ok(Self {
            voice: file.voice.unwrap_or_else(|| DEFAULT_VOICE.to_string()),
            speed: file.speed.unwrap_or(DEFAULT_SPEED),
            idle_timeout: Duration::from_secs(idle_minutes * 60),
            home,
        })
    }

    pub fn model_path(&self) -> PathBuf {
        self.home.join(MODEL_FILE)
    }

    pub fn voices_path(&self) -> PathBuf {
        self.home.join(VOICES_FILE)
    }

    /// Unix socket the daemon binds and the hook client connects to.
    pub fn socket_path(&self) -> PathBuf {
        self.home.join(SOCKET_FILE)
    }
}

/// Write a default `config.toml` under `home`, **only if absent** — never clobber
/// a user-edited file. Returns `true` if it wrote, `false` if one already existed.
/// Creates `home` if needed. (Called by `setup::init`.)
pub fn write_default_config(home: &Path) -> Result<bool> {
    std::fs::create_dir_all(home).with_context(|| format!("create {home:?}"))?;
    let path = home.join(CONFIG_FILE);
    // `create_new` is race-free: it atomically fails if the file already exists,
    // so a config created between a check and a write can never be clobbered.
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("create {path:?}")),
    };
    let defaults = ConfigFile {
        voice: Some(DEFAULT_VOICE.to_string()),
        speed: Some(DEFAULT_SPEED),
        idle_timeout_minutes: Some(DEFAULT_IDLE_MINUTES),
    };
    let toml = toml::to_string(&defaults).context("serialize default config")?;
    file.write_all(toml.as_bytes())
        .with_context(|| format!("write {path:?}"))?;
    file.sync_all().with_context(|| format!("sync {path:?}"))?;
    Ok(true)
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
    fn parses_idle_timeout() {
        let dir = scratch("idle");
        std::fs::write(dir.join(CONFIG_FILE), "idle_timeout_minutes = 5\n").unwrap();
        let file = ConfigFile::read(&dir).unwrap();
        assert_eq!(file.idle_timeout_minutes, Some(5));
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

    #[test]
    fn writes_default_when_absent() {
        let dir = scratch("write-new");
        let _ = std::fs::remove_file(dir.join(CONFIG_FILE));
        assert!(write_default_config(&dir).unwrap());
        // What we wrote must round-trip back to the defaults.
        let file = ConfigFile::read(&dir).unwrap();
        assert_eq!(file.voice.as_deref(), Some(DEFAULT_VOICE));
        assert_eq!(file.speed, Some(DEFAULT_SPEED));
        assert_eq!(file.idle_timeout_minutes, Some(DEFAULT_IDLE_MINUTES));
    }

    #[test]
    fn does_not_clobber_existing_config() {
        let dir = scratch("write-keep");
        std::fs::write(dir.join(CONFIG_FILE), "voice = \"am_adam\"\n").unwrap();
        assert!(!write_default_config(&dir).unwrap());
        assert_eq!(
            ConfigFile::read(&dir).unwrap().voice.as_deref(),
            Some("am_adam")
        );
    }
}
