//! Speech on/off, as a flag file under the koklaude home.
//!
//! Presence of the `enabled` file = on. Fresh install is off until enabled
//! (`koklaude on`, or `init` in Phase 5). The hook reads `is_enabled` to decide
//! whether to speak at all.

use std::path::Path;

use anyhow::{Context, Result};

const FLAG: &str = "enabled";

/// Is speech currently enabled?
// Used by the hook (Phase 4); remove this allow when wired.
#[allow(dead_code)]
pub fn is_enabled(home: &Path) -> bool {
    home.join(FLAG).exists()
}

/// Turn speech on (idempotent). Creates the home dir if needed.
pub fn enable(home: &Path) -> Result<()> {
    std::fs::create_dir_all(home).with_context(|| format!("create {home:?}"))?;
    let flag = home.join(FLAG);
    std::fs::write(&flag, b"").with_context(|| format!("write {flag:?}"))
}

/// Turn speech off (idempotent — already-off is fine).
pub fn disable(home: &Path) -> Result<()> {
    let flag = home.join(FLAG);
    match std::fs::remove_file(&flag) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {flag:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-toggle-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn enable_then_disable_round_trip() {
        let dir = scratch("roundtrip");
        assert!(!is_enabled(&dir)); // fresh: off
        enable(&dir).unwrap();
        assert!(is_enabled(&dir));
        disable(&dir).unwrap();
        assert!(!is_enabled(&dir));
    }

    #[test]
    fn enable_creates_missing_home() {
        let dir = scratch("makedir").join("nested");
        enable(&dir).unwrap();
        assert!(is_enabled(&dir));
    }

    #[test]
    fn disable_when_off_is_ok() {
        let dir = scratch("idempotent");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(disable(&dir).is_ok());
    }

    #[test]
    fn enable_is_idempotent() {
        let dir = scratch("double");
        enable(&dir).unwrap();
        enable(&dir).unwrap();
        assert!(is_enabled(&dir));
    }
}
