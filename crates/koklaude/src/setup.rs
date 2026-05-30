//! Install/uninstall surgery on Claude Code's `~/.claude/settings.json`.
//!
//! Pure JSON transforms — no filesystem here (5d owns read + atomic write). Add
//! or remove koklaude's Stop hook while preserving every other hook the user has.
//! Verified schema (code.claude.com/docs hooks): `hooks.Stop` is an array of
//! groups, each `{ "hooks": [ { "type": "command", "command": "…" } ] }`. Stop
//! has **no `matcher`** — it fires unconditionally.
// Unwired until 5d composes `init`/`uninstall`; drop this then.
#![allow(dead_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// Release assets to fetch — kokoro-onnx `model-files-v1.0`; see
/// docs/prerequisites.md. (5d pairs each with its `Config` dest path.)
pub const MODEL_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";
pub const VOICES_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";

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
    let res = ureq::get(url).call().with_context(|| format!("GET {url}"))?;
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
        file.write_all(&buf[..n]).with_context(|| format!("write {part:?}"))?;
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
        assert!(!part_path(&dest).exists(), ".part must be renamed away on success");
    }

    #[test]
    fn download_skips_when_dest_present() {
        let dest = scratch("skip").join("present.bin");
        std::fs::write(&dest, b"x").unwrap();
        // Bogus URL: if the skip works, it's never fetched, so this can't error.
        download("http://invalid.invalid/nope", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"x");
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
            let round = remove_stop_hook(merge_stop_hook(original.clone(), CMD).unwrap(), CMD).unwrap();
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
