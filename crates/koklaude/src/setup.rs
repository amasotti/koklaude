//! Install/uninstall surgery on Claude Code's `~/.claude/settings.json`.
//!
//! Pure JSON transforms — no filesystem here (5d owns read + atomic write). Add
//! or remove koklaude's Stop hook while preserving every other hook the user has.
//! Verified schema (code.claude.com/docs hooks): `hooks.Stop` is an array of
//! groups, each `{ "hooks": [ { "type": "command", "command": "…" } ] }`. Stop
//! has **no `matcher`** — it fires unconditionally.

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
    stream_to_file(body.into_reader(), dest, total)
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

/// Add koklaude's Stop hook to `settings`, preserving existing hooks. Idempotent:
/// if `command` is already registered under any Stop group, returns unchanged.
/// Errors (rather than clobber) when `hooks`/`Stop` exist with the wrong shape.
pub fn merge_stop_hook(mut settings: Value, command: &str) -> Result<Value> {
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
    if !stop_contains(stop, command) {
        stop.push(json!({ "hooks": [{ "type": "command", "command": command }] }));
    }
    Ok(settings)
}

/// Remove koklaude's Stop hook from `settings`, leaving every other hook intact.
/// Strips `command` from each Stop group and cascade-cleans anything it empties
/// (group → `Stop` → `hooks`), so `merge` then `remove` restores the original.
pub fn remove_stop_hook(mut settings: Value, command: &str) -> Result<Value> {
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

    // Drop our command from every group; drop a group that ends up empty.
    // Foreign group shapes are left untouched.
    stop.retain_mut(|group| match group.get_mut("hooks") {
        Some(Value::Array(inner)) => {
            inner.retain(|h| h.get("command").and_then(Value::as_str) != Some(command));
            !inner.is_empty()
        }
        _ => true,
    });

    // Cascade-clean emptied containers so an uninstall leaves no husk behind.
    if stop.is_empty() {
        hooks.remove("Stop");
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(settings)
}

/// Is `command` already registered under any Stop group?
fn stop_contains(stop: &[Value], command: &str) -> bool {
    stop.iter().any(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|inner| {
                inner
                    .iter()
                    .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
            })
    })
}

// --- composition: `koklaude init` / `koklaude uninstall` -----------------

/// One-command setup: detect espeak → fetch model + voices → write config →
/// register the Stop hook → enable. Idempotent — re-running fills only what's
/// missing (downloads skip, config is kept, the hook merge dedupes).
pub fn init(cfg: &Config) -> Result<()> {
    if !espeak_installed() {
        eprintln!("⚠ espeak-ng not on PATH — koklaude can't speak until it's installed:");
        eprintln!("    brew install espeak-ng        # macOS");
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

    let command = hook_command()?;
    let settings = claude_settings_path()?;
    rewrite_settings(&settings, |s| merge_stop_hook(s, &command))?;
    println!("registered Stop hook in {}", settings.display());

    toggle::enable(&cfg.home)?;
    println!("✓ koklaude ready — Claude will speak its replies.");
    Ok(())
}

/// Remove koklaude's Stop hook and disable speech, leaving every other Claude Code
/// hook intact. With `purge`, also delete the koklaude home (model, voices, config)
/// — off by default, since re-downloading is expensive.
pub fn uninstall(home: &Path, purge: bool) -> Result<()> {
    let command = hook_command()?;
    let settings = claude_settings_path()?;
    if settings.exists() {
        rewrite_settings(&settings, |s| remove_stop_hook(s, &command))?;
        println!("removed Stop hook from {}", settings.display());
    }
    toggle::disable(home)?;
    if purge && home.exists() {
        std::fs::remove_dir_all(home).with_context(|| format!("remove {home:?}"))?;
        println!("purged {}", home.display());
    }
    println!("✓ koklaude uninstalled.");
    Ok(())
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

/// The command we register as the Stop hook: this binary's absolute path + `hook`
/// (so the hook works regardless of `$PATH`).
fn hook_command() -> Result<String> {
    let exe = std::env::current_exe().context("locate the koklaude binary")?;
    Ok(format!("{} hook", exe.display()))
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
    let bytes = serde_json::to_vec_pretty(&updated).context("serialize settings")?;
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

    const CMD: &str = "koklaude hook";

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

        rewrite_settings(&path, |s| merge_stop_hook(s, CMD)).unwrap();
        let reg: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reg["model"], "opus"); // unrelated keys preserved
        assert!(stop_contains(reg["hooks"]["Stop"].as_array().unwrap(), CMD));

        rewrite_settings(&path, |s| remove_stop_hook(s, CMD)).unwrap();
        let unreg: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(unreg, json!({ "model": "opus" })); // byte-restored
    }

    #[test]
    fn rewrite_creates_file_when_absent() {
        let path = scratch("settings-new").join("settings.json");
        let _ = std::fs::remove_file(&path);
        rewrite_settings(&path, |s| merge_stop_hook(s, CMD)).unwrap();
        assert!(path.exists());
        assert!(!part_path(&path).exists()); // temp renamed away
    }

    #[test]
    fn rewrite_refuses_to_clobber_malformed_settings() {
        let path = scratch("settings-bad").join("settings.json");
        std::fs::write(&path, "not json {{{").unwrap();
        assert!(rewrite_settings(&path, |s| merge_stop_hook(s, CMD)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json {{{"); // untouched
    }

    // --- merge -----------------------------------------------------------

    #[test]
    fn merge_into_empty_settings() {
        let got = merge_stop_hook(json!({}), CMD).unwrap();
        assert_eq!(
            got,
            json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": CMD }] }] } })
        );
    }

    #[test]
    fn merge_preserves_unrelated_hooks() {
        let before = json!({
            "model": "opus",
            "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "log.sh" }] }] }
        });
        let got = merge_stop_hook(before, CMD).unwrap();
        assert_eq!(got["model"], "opus");
        assert!(got["hooks"]["PreToolUse"].is_array());
        assert!(stop_contains(got["hooks"]["Stop"].as_array().unwrap(), CMD));
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge_stop_hook(json!({}), CMD).unwrap();
        let twice = merge_stop_hook(once.clone(), CMD).unwrap();
        assert_eq!(once, twice);
        assert_eq!(twice["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_appends_alongside_user_stop_hook() {
        let before = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] }
        });
        let got = merge_stop_hook(before, CMD).unwrap();
        let stop = got["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert!(stop_contains(stop, CMD));
        assert!(stop_contains(stop, "mine.sh"));
    }

    #[test]
    fn merge_errors_on_wrong_shapes() {
        assert!(merge_stop_hook(json!(42), CMD).is_err());
        assert!(merge_stop_hook(json!({ "hooks": 5 }), CMD).is_err());
        assert!(merge_stop_hook(json!({ "hooks": { "Stop": 5 } }), CMD).is_err());
    }

    // --- remove ----------------------------------------------------------

    #[test]
    fn remove_cleans_up_completely() {
        let with = merge_stop_hook(json!({}), CMD).unwrap();
        let got = remove_stop_hook(with, CMD).unwrap();
        assert_eq!(got, json!({})); // emptied group → Stop → hooks all pruned
    }

    #[test]
    fn remove_preserves_other_stop_groups() {
        let before = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] }
        });
        let with = merge_stop_hook(before.clone(), CMD).unwrap();
        let got = remove_stop_hook(with, CMD).unwrap();
        assert_eq!(got, before); // user's hook survives untouched
    }

    #[test]
    fn remove_is_noop_when_absent() {
        let settings = json!({ "hooks": { "PreToolUse": [] } });
        assert_eq!(remove_stop_hook(settings.clone(), CMD).unwrap(), settings);
        assert_eq!(remove_stop_hook(json!({}), CMD).unwrap(), json!({}));
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
                remove_stop_hook(merge_stop_hook(original.clone(), CMD).unwrap(), CMD).unwrap();
            assert_eq!(round, original, "round-trip must restore the original");
        }
    }

    #[test]
    fn remove_errors_on_wrong_shapes() {
        assert!(remove_stop_hook(json!(42), CMD).is_err());
        assert!(remove_stop_hook(json!({ "hooks": 5 }), CMD).is_err());
        assert!(remove_stop_hook(json!({ "hooks": { "Stop": 5 } }), CMD).is_err());
    }
}
