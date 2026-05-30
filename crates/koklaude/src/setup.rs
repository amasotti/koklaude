//! Install/uninstall surgery on Claude Code's `~/.claude/settings.json`.
//!
//! Add or remove koklaude's Stop hook while preserving every other hook the user
//! has. Verified schema (code.claude.com/docs hooks): `hooks.Stop` is an array of
//! groups, each `{ "hooks": [ <entry> ] }`. We register the entry in **exec form**
//! — `{ "type": "command", "command": "<abs path>", "args": ["hook"] }` — which
//! Claude Code spawns directly with no shell, so a binary path containing spaces
//! needs no quoting. Stop has **no `matcher`** — it fires unconditionally.
//!
//! Our hook is identified **structurally** (binary basename `koklaude` + args
//! `["hook"]`), not by exact command string, so a re-install from a different
//! path replaces rather than duplicates, and uninstall always finds it.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::config::{Config, write_default_config};
use crate::toggle;

/// Release assets to fetch — kokoro-onnx `model-files-v1.0`; see
/// docs/prerequisites.md. (5d pairs each with its `Config` dest path.)
pub const MODEL_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";
pub const VOICES_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";

const DOWNLOAD_BUF: usize = 64 * 1024;
/// Redraw the stderr progress line roughly every this many bytes.
const PROGRESS_STEP: u64 = 8 * 1024 * 1024;

/// Download `url` to `dest`. Skips if `dest` already exists non-empty (re-download
/// is expensive). Streams to a `<dest>.part` sibling then renames on success — an
/// interrupted download never leaves a truncated file at `dest`. Progress → stderr.
pub fn download(url: &str, dest: &Path) -> Result<()> {
    if dest.metadata().is_ok_and(|m| m.len() > 0) {
        eprintln!("  {} present — skipping", dest.display());
        return Ok(());
    }
    let res = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let body = res.into_body();
    let total = body.content_length();
    let result = stream_to_file(body.into_reader(), dest, total);
    if result.is_err() {
        // A truncated/failed stream must not leave a stale `.part` lying around.
        let _ = std::fs::remove_file(part_path(dest));
    }
    result
}

/// Stream `reader` into `dest` via a `.part` temp + atomic rename. Split out from
/// `download` so the file plumbing is testable without a network round-trip.
fn stream_to_file(mut reader: impl Read, dest: &Path, total: Option<u64>) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
    }
    let part = part_path(dest);
    let name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("file");

    let mut file = File::create(&part).with_context(|| format!("create {part:?}"))?;
    let mut buf = vec![0u8; DOWNLOAD_BUF];
    let (mut done, mut drawn): (u64, u64) = (0, 0);
    loop {
        let n = reader.read(&mut buf).context("read download stream")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("write {part:?}"))?;
        done += n as u64;
        if done - drawn >= PROGRESS_STEP {
            draw_progress(name, done, total);
            drawn = done;
        }
    }
    file.sync_all().with_context(|| format!("flush {part:?}"))?;
    drop(file);
    draw_progress(name, done, total);
    eprintln!();

    std::fs::rename(&part, dest).with_context(|| format!("rename {part:?} -> {dest:?}"))
}

/// `<dest>.part` — append, don't replace the extension (`x.onnx` → `x.onnx.part`).
fn part_path(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_owned();
    p.push(".part");
    PathBuf::from(p)
}

fn draw_progress(name: &str, done: u64, total: Option<u64>) {
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    match total {
        Some(t) if t > 0 => eprint!(
            "\r  {name}: {:.1} / {:.1} MB ({:.0}%)",
            mb(done),
            mb(t),
            done as f64 / t as f64 * 100.0
        ),
        _ => eprint!("\r  {name}: {:.1} MB", mb(done)),
    }
}

/// Is `espeak-ng` on PATH? koklaude shells out to it for g2p (decisions D3), so
/// `init` checks this up front and prints the `brew install espeak-ng` hint when
/// it's missing. Output is silenced — we only care about the exit status.
pub fn espeak_installed() -> bool {
    Command::new("espeak-ng")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Register koklaude's Stop hook for binary `exe`, preserving existing hooks.
/// Idempotent and self-healing: strips any prior koklaude hook first (even one
/// registered from a different path), so re-running never duplicates. Errors
/// (rather than clobber) when `hooks`/`Stop` exist with the wrong shape.
pub fn merge_stop_hook(mut settings: Value, exe: &Path) -> Result<Value> {
    let Value::Object(root) = &mut settings else {
        bail!("settings is not a JSON object");
    };
    let hooks = match root.entry("hooks").or_insert_with(|| json!({})) {
        Value::Object(m) => m,
        _ => bail!("`hooks` is present but is not a JSON object"),
    };
    let stop = match hooks.entry("Stop").or_insert_with(|| json!([])) {
        Value::Array(a) => a,
        _ => bail!("`Stop` is present but is not a JSON array"),
    };
    strip_our_hooks(stop);
    stop.push(hook_group(exe));
    Ok(settings)
}

/// Remove koklaude's Stop hook(s) from `settings`, leaving every other hook intact.
/// Matches **structurally** (binary basename `koklaude` + args `["hook"]`), so it
/// finds our hook regardless of which path registered it. Cascade-cleans anything
/// it empties (group → `Stop` → `hooks`), so `merge` then `remove` restores the
/// original.
pub fn remove_stop_hook(mut settings: Value) -> Result<Value> {
    let Value::Object(root) = &mut settings else {
        bail!("settings is not a JSON object");
    };
    let hooks = match root.get_mut("hooks") {
        None => return Ok(settings),
        Some(Value::Object(m)) => m,
        Some(_) => bail!("`hooks` is present but is not a JSON object"),
    };
    let stop = match hooks.get_mut("Stop") {
        None => return Ok(settings),
        Some(Value::Array(a)) => a,
        Some(_) => bail!("`Stop` is present but is not a JSON array"),
    };

    strip_our_hooks(stop);
    if stop.is_empty() {
        hooks.remove("Stop");
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(settings)
}

/// Drop every koklaude hook from each Stop group; drop a group left empty. Foreign
/// group shapes are left untouched.
fn strip_our_hooks(stop: &mut Vec<Value>) {
    stop.retain_mut(|group| match group.get_mut("hooks") {
        Some(Value::Array(inner)) => {
            inner.retain(|h| !is_koklaude_hook(h));
            !inner.is_empty()
        }
        _ => true,
    });
}

/// A fresh Stop group registering `exe` in exec form (no shell → spaces safe).
fn hook_group(exe: &Path) -> Value {
    json!({ "hooks": [{ "type": "command", "command": exe.to_string_lossy(), "args": ["hook"] }] })
}

/// Is this inner hook entry one of ours? Identified by the binary's basename
/// (`koklaude`) plus the `["hook"]` args — independent of the absolute path, so a
/// move/reinstall can't orphan it.
fn is_koklaude_hook(entry: &Value) -> bool {
    let exe_is_koklaude = entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| Path::new(c).file_name() == Some(OsStr::new("koklaude")));
    let args_is_hook = entry
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|a| a.len() == 1 && a.first().and_then(Value::as_str) == Some("hook"));
    exe_is_koklaude && args_is_hook
}

// --- composition: `koklaude init` / `koklaude uninstall` -----------------

/// One-command setup: detect espeak → fetch model + voices → write config →
/// register the Stop hook → enable. Idempotent — re-running fills only what's
/// missing (downloads skip, config is kept, the hook merge dedupes).
pub fn init(cfg: &Config) -> Result<()> {
    let espeak = espeak_installed();
    if !espeak {
        eprintln!("⚠ espeak-ng not on PATH — koklaude can't speak until it's installed:");
        eprintln!("    brew install espeak-ng         # macOS");
        eprintln!("    sudo apt-get install espeak-ng # Debian/Ubuntu");
    }
    std::fs::create_dir_all(&cfg.home).with_context(|| format!("create {:?}", cfg.home))?;

    println!("fetching model + voices into {}…", cfg.home.display());
    download(MODEL_URL, &cfg.model_path())?;
    download(VOICES_URL, &cfg.voices_path())?;

    if write_default_config(&cfg.home)? {
        println!("wrote default config.toml");
    } else {
        println!("kept existing config.toml");
    }

    let exe = std::env::current_exe().context("locate the koklaude binary")?;
    let settings = claude_settings_path()?;
    rewrite_settings(&settings, |s| merge_stop_hook(s, &exe))?;
    println!("registered Stop hook in {}", settings.display());

    toggle::enable(&cfg.home)?;
    // Don't claim "ready" if the engine can't actually run.
    if espeak {
        println!("✓ koklaude ready — Claude will speak its replies.");
    } else {
        println!("✓ koklaude installed — install espeak-ng (above) to activate speech.");
    }
    Ok(())
}

/// Remove koklaude's Stop hook and disable speech, leaving every other Claude Code
/// hook intact. With `purge`, also delete the koklaude home (model, voices, config)
/// — off by default, since re-downloading is expensive.
pub fn uninstall(home: &Path, purge: bool) -> Result<()> {
    let settings = claude_settings_path()?;
    if settings.exists() {
        rewrite_settings(&settings, remove_stop_hook)?;
        println!("removed Stop hook from {}", settings.display());
    }
    toggle::disable(home)?;
    if purge {
        purge_home(home)?;
        println!("purged {}", home.display());
    }
    println!("✓ koklaude uninstalled.");
    Ok(())
}

/// Delete the koklaude home — but **only** a directory whose final path component
/// is exactly `koklaude`, and only if it's a real directory (not a symlink). This
/// is a hard safety wall: no matter what the user points `$KOKLAUDE_HOME` at, we
/// will never `remove_dir_all` a parent, a home dir, `/`, or a symlink's target.
fn purge_home(home: &Path) -> Result<()> {
    if home.file_name() != Some(OsStr::new("koklaude")) {
        bail!(
            "refusing to purge {home:?}: final path component must be exactly \"koklaude\" \
             (delete it yourself if that's really what you want)"
        );
    }
    match std::fs::symlink_metadata(home) {
        Ok(m) if m.file_type().is_symlink() => {
            bail!("refusing to purge {home:?}: it is a symlink, not a real directory");
        }
        Ok(m) if !m.is_dir() => bail!("refusing to purge {home:?}: not a directory"),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), // nothing to purge
        Err(e) => return Err(e).with_context(|| format!("stat {home:?}")),
    }
    std::fs::remove_dir_all(home).with_context(|| format!("remove {home:?}"))
}

/// Path to Claude Code's `settings.json` — `$CLAUDE_CONFIG_DIR` or `~/.claude`.
fn claude_settings_path() -> Result<PathBuf> {
    let dir = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::home_dir()
            .context("locate home directory")?
            .join(".claude"),
    };
    Ok(dir.join("settings.json"))
}

/// Read `path` (or `{}` if absent), apply `f`, write back atomically. A malformed
/// existing file errors out rather than being clobbered.
fn rewrite_settings(path: &Path, f: impl FnOnce(Value) -> Result<Value>) -> Result<()> {
    let current = match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).with_context(|| format!("parse {path:?}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e).with_context(|| format!("read {path:?}")),
    };
    let updated = f(current)?;
    let mut bytes = serde_json::to_vec_pretty(&updated).context("serialize settings")?;
    bytes.push(b'\n'); // trailing newline — POSIX text file, no spurious git diff
    atomic_write(path, &bytes)
}

/// Write `bytes` to `path` via a sibling `.part` temp + rename — a crash mid-write
/// can never corrupt the user's existing `settings.json`.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
    }
    let tmp = part_path(path);
    std::fs::write(&tmp, bytes).with_context(|| format!("write {tmp:?}"))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename {tmp:?} -> {path:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const EXE: &str = "/usr/local/bin/koklaude";

    /// Every koklaude hook entry across all Stop groups in `settings`.
    fn our_hooks(settings: &Value) -> Vec<&Value> {
        settings
            .get("hooks")
            .and_then(|h| h.get("Stop"))
            .and_then(Value::as_array)
            .map(|stop| {
                stop.iter()
                    .filter_map(|g| g.get("hooks").and_then(Value::as_array))
                    .flatten()
                    .filter(|h| is_koklaude_hook(h))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Per-test scratch dir under temp (no env mutation → no test races).
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-setup-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- download plumbing (no network) ----------------------------------

    #[test]
    fn part_path_appends_not_replaces_extension() {
        assert_eq!(
            part_path(Path::new("/a/kokoro-v1.0.onnx")),
            PathBuf::from("/a/kokoro-v1.0.onnx.part")
        );
    }

    #[test]
    fn stream_writes_content_and_renames_part_away() {
        let dest = scratch("stream").join("asset.bin");
        let _ = std::fs::remove_file(&dest);
        let data = b"hello kokoro".to_vec();
        stream_to_file(Cursor::new(data.clone()), &dest, None).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(
            !part_path(&dest).exists(),
            ".part must be renamed away on success"
        );
    }

    /// Network-gated: proves the real `ureq` fetch path end to end (the Cursor
    /// tests exercise only the file plumbing). Hits a small file, not the 340 MB
    /// assets. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "network"]
    fn download_fetches_a_real_file() {
        let dest = scratch("net").join("LICENSE");
        let _ = std::fs::remove_file(&dest);
        download(
            "https://raw.githubusercontent.com/thewh1teagle/kokoro-onnx/main/LICENSE",
            &dest,
        )
        .unwrap();
        assert!(std::fs::metadata(&dest).unwrap().len() > 0);
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn download_skips_when_dest_present() {
        let dest = scratch("skip").join("present.bin");
        std::fs::write(&dest, b"x").unwrap();
        // Bogus URL: if the skip works, it's never fetched, so this can't error.
        download("http://invalid.invalid/nope", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"x");
    }

    // --- settings.json rewrite (no network) ------------------------------

    #[test]
    fn register_then_unregister_restores_settings() {
        let path = scratch("settings").join("settings.json");
        std::fs::write(&path, r#"{"model":"opus"}"#).unwrap();

        rewrite_settings(&path, |s| merge_stop_hook(s, Path::new(EXE))).unwrap();
        let reg: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reg["model"], "opus"); // unrelated keys preserved
        assert_eq!(our_hooks(&reg).len(), 1);

        rewrite_settings(&path, remove_stop_hook).unwrap();
        let unreg: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(unreg, json!({ "model": "opus" })); // semantically restored
    }

    #[test]
    fn rewrite_creates_file_when_absent() {
        let path = scratch("settings-new").join("settings.json");
        let _ = std::fs::remove_file(&path);
        rewrite_settings(&path, |s| merge_stop_hook(s, Path::new(EXE))).unwrap();
        assert!(path.exists());
        assert!(!part_path(&path).exists()); // temp renamed away
    }

    #[test]
    fn rewrite_refuses_to_clobber_malformed_settings() {
        let path = scratch("settings-bad").join("settings.json");
        std::fs::write(&path, "not json {{{").unwrap();
        assert!(rewrite_settings(&path, |s| merge_stop_hook(s, Path::new(EXE))).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json {{{"); // untouched
    }

    // --- merge -----------------------------------------------------------

    #[test]
    fn merge_into_empty_settings_uses_exec_form() {
        let got = merge_stop_hook(json!({}), Path::new(EXE)).unwrap();
        assert_eq!(
            got,
            json!({ "hooks": { "Stop": [{ "hooks": [
                { "type": "command", "command": EXE, "args": ["hook"] }
            ] }] } })
        );
    }

    #[test]
    fn merge_preserves_unrelated_hooks() {
        let before = json!({
            "model": "opus",
            "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "log.sh" }] }] }
        });
        let got = merge_stop_hook(before, Path::new(EXE)).unwrap();
        assert_eq!(got["model"], "opus");
        assert!(got["hooks"]["PreToolUse"].is_array());
        assert_eq!(our_hooks(&got).len(), 1);
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge_stop_hook(json!({}), Path::new(EXE)).unwrap();
        let twice = merge_stop_hook(once.clone(), Path::new(EXE)).unwrap();
        assert_eq!(once, twice);
        assert_eq!(our_hooks(&twice).len(), 1);
    }

    /// #3 regression: re-install from a *different* binary path must REPLACE the
    /// old koklaude hook, never duplicate it (would make Claude speak twice).
    #[test]
    fn merge_reinstall_from_new_path_replaces_not_duplicates() {
        let first = merge_stop_hook(json!({}), Path::new("/old/path/koklaude")).unwrap();
        let second = merge_stop_hook(first, Path::new("/new/place/koklaude")).unwrap();
        let ours = our_hooks(&second);
        assert_eq!(ours.len(), 1, "exactly one koklaude hook after reinstall");
        assert_eq!(ours[0]["command"], "/new/place/koklaude"); // the current path won
    }

    /// #2 regression: a binary path with spaces is stored verbatim (exec form,
    /// no shell, no quoting) and still recognized as ours.
    #[test]
    fn merge_handles_binary_path_with_spaces() {
        let exe = Path::new("/Users/some user/.cargo/bin/koklaude");
        let got = merge_stop_hook(json!({}), exe).unwrap();
        let ours = our_hooks(&got);
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0]["command"], "/Users/some user/.cargo/bin/koklaude");
        assert_eq!(ours[0]["args"], json!(["hook"]));
    }

    #[test]
    fn merge_keeps_a_foreign_stop_hook() {
        let before = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] }
        });
        let got = merge_stop_hook(before, Path::new(EXE)).unwrap();
        let stop = got["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2); // foreign group + ours
        assert_eq!(our_hooks(&got).len(), 1);
    }

    #[test]
    fn merge_errors_on_wrong_shapes() {
        assert!(merge_stop_hook(json!(42), Path::new(EXE)).is_err());
        assert!(merge_stop_hook(json!({ "hooks": 5 }), Path::new(EXE)).is_err());
        assert!(merge_stop_hook(json!({ "hooks": { "Stop": 5 } }), Path::new(EXE)).is_err());
    }

    // --- remove ----------------------------------------------------------

    #[test]
    fn remove_cleans_up_completely() {
        let with = merge_stop_hook(json!({}), Path::new(EXE)).unwrap();
        let got = remove_stop_hook(with).unwrap();
        assert_eq!(got, json!({})); // emptied group → Stop → hooks all pruned
    }

    #[test]
    fn remove_preserves_other_stop_groups() {
        let before = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] }
        });
        let with = merge_stop_hook(before.clone(), Path::new(EXE)).unwrap();
        let got = remove_stop_hook(with).unwrap();
        assert_eq!(got, before); // user's hook survives untouched
    }

    /// #3 regression: remove finds our hook structurally, so a hook registered
    /// from one path is removed even though no path/command is passed in.
    #[test]
    fn remove_is_orphan_proof_across_paths() {
        let with = merge_stop_hook(json!({}), Path::new("/some/old/koklaude")).unwrap();
        assert_eq!(remove_stop_hook(with).unwrap(), json!({}));
    }

    #[test]
    fn remove_is_noop_when_absent() {
        let settings = json!({ "hooks": { "PreToolUse": [] } });
        assert_eq!(remove_stop_hook(settings.clone()).unwrap(), settings);
        assert_eq!(remove_stop_hook(json!({})).unwrap(), json!({}));
    }

    #[test]
    fn merge_then_remove_round_trips() {
        for original in [
            json!({}),
            json!({ "model": "opus" }),
            json!({ "hooks": { "PreToolUse": [{ "hooks": [{ "type": "command", "command": "x.sh" }] }] } }),
            json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] } }),
        ] {
            let round =
                remove_stop_hook(merge_stop_hook(original.clone(), Path::new(EXE)).unwrap())
                    .unwrap();
            assert_eq!(round, original, "round-trip must restore the original");
        }
    }

    #[test]
    fn remove_errors_on_wrong_shapes() {
        assert!(remove_stop_hook(json!(42)).is_err());
        assert!(remove_stop_hook(json!({ "hooks": 5 })).is_err());
        assert!(remove_stop_hook(json!({ "hooks": { "Stop": 5 } })).is_err());
    }

    // --- purge guard (the hard safety wall) ------------------------------

    #[test]
    fn purge_removes_a_real_koklaude_dir() {
        let home = scratch("purge-ok").join("koklaude");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("sub")).unwrap();
        std::fs::write(home.join("kokoro-v1.0.onnx"), b"x").unwrap();
        purge_home(&home).unwrap();
        assert!(!home.exists());
    }

    #[test]
    fn purge_refuses_dir_not_named_koklaude() {
        let home = scratch("purge-bad").join("not-koklaude");
        std::fs::create_dir_all(&home).unwrap();
        assert!(purge_home(&home).is_err());
        assert!(home.exists(), "must NOT delete a non-koklaude directory");
    }

    #[test]
    fn purge_refuses_home_root_and_parents() {
        // Final component is the user's home / a parent — never koklaude → refused.
        assert!(purge_home(Path::new("/Users/toni")).is_err());
        assert!(purge_home(Path::new("/")).is_err());
        assert!(purge_home(Path::new("/Users/toni/.config")).is_err());
    }

    #[test]
    fn purge_is_noop_when_absent() {
        let home = scratch("purge-absent").join("koklaude");
        let _ = std::fs::remove_dir_all(&home);
        purge_home(&home).unwrap(); // nothing to remove → Ok
    }

    /// Even if a symlink is *named* `koklaude`, we must not follow it and nuke
    /// its target.
    #[cfg(unix)]
    #[test]
    fn purge_refuses_symlink_named_koklaude() {
        let base = scratch("purge-symlink");
        let target = base.join("precious");
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.txt"), b"keep").unwrap();
        let link = base.join("koklaude");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(purge_home(&link).is_err());
        assert!(
            target.join("keep.txt").exists(),
            "symlink target must survive"
        );
    }
}
