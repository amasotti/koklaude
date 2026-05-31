//! Install/uninstall surgery on Claude Code's `~/.claude/settings.json`.
//!
//! Add or remove koklaude's Stop hook while preserving every other hook the user
//! has. Verified schema (code.claude.com/docs hooks): `hooks.Stop` is an array of
//! groups, each `{ "hooks": [ <entry> ] }`. We register the entry in **exec form**
//! — `{ "type": "command", "command": "<abs path>", "args": ["hook"] }` — which
//! Claude Code spawns directly with no shell, so a binary path containing spaces
//! needs no quoting. Stop has **no `matcher`** — it fires unconditionally.
//!
//! Our hook is identified **structurally** (the `koklaude` binary invoked with the
//! `hook` subcommand — exec *or* shell form), not by exact command string, so a
//! re-install from a different path replaces rather than duplicates, and uninstall
//! always finds it — including a hand-edited or corrupt entry. See
//! `is_koklaude_hook`.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tracing::info;
use ureq::Agent;
use ureq::config::{Config as UreqConfig, IpFamily};

use crate::config::{Config, write_default_config};
use crate::toggle;

/// Assets are fetched from the official community ONNX repo on Hugging Face,
/// `onnx-community/Kokoro-82M-v1.0-ONNX` (model `onnx/model.onnx`; voices as
/// per-file `voices/<name>.bin`). See docs/prerequisites.md.
const HF_BASE: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main";

/// Every voice in the pinned v1.0 repo — one `voices/<name>.bin` each. `init`
/// downloads all of them so any voice works offline (`say --voice …`).
pub const VOICES: &[&str] = &[
    "af",
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
    "ef_dora",
    "em_alex",
    "em_santa",
    "ff_siwis",
    "hf_alpha",
    "hf_beta",
    "hm_omega",
    "hm_psi",
    "if_sara",
    "im_nicola",
    "jf_alpha",
    "jf_gongitsune",
    "jf_nezumi",
    "jf_tebukuro",
    "jm_kumo",
    "pf_dora",
    "pm_alex",
    "pm_santa",
    "zf_xiaobei",
    "zf_xiaoni",
    "zf_xiaoxiao",
    "zf_xiaoyi",
    "zm_yunjian",
    "zm_yunxi",
    "zm_yunxia",
    "zm_yunyang",
];

/// URL for the Kokoro ONNX model weights.
fn model_url() -> String {
    format!("{HF_BASE}/onnx/model.onnx")
}

/// URL for voice `name`'s style file.
fn voice_url(name: &str) -> String {
    format!("{HF_BASE}/voices/{name}.bin")
}

const DOWNLOAD_BUF: usize = 64 * 1024;
/// Redraw the stderr progress line roughly every this many bytes.
const PROGRESS_STEP: u64 = 8 * 1024 * 1024;
/// Only stream byte-progress for files at least this large (skips it for the
/// tiny per-voice `.bin`s — `init` prints a `[i/N]` line for those instead).
const PROGRESS_MIN_BYTES: u64 = 4 * 1024 * 1024;

/// HTTP client for fetching from Hugging Face.
///
/// `Ipv4Only`: HF's IPv6 endpoints black-hole on some networks, and ureq has no
/// Happy-Eyeballs fallback, so the default `Any` picks the dead IPv6 address and
/// (without a connect timeout) hangs forever. We only ever talk to HF, so pinning
/// IPv4 is safe. The timeouts are a belt-and-braces guarantee against any silent
/// stall. One agent is reused across all files (connection pooling).
fn http_agent() -> Agent {
    UreqConfig::builder()
        .ip_family(IpFamily::Ipv4Only)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Download `url` to `dest` via `agent`. Returns `false` (and does nothing) if
/// `dest` already exists non-empty — re-download is expensive. Streams to a
/// `<dest>.part` sibling then renames on success, so an interrupted download
/// never leaves a truncated file at `dest`. Progress → stderr (large files only).
pub fn download(agent: &Agent, url: &str, dest: &Path) -> Result<bool> {
    if dest.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(false);
    }
    let res = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let body = res.into_body();
    let total = body.content_length();
    let result = stream_to_file(body.into_reader(), dest, total);
    if result.is_err() {
        // A truncated/failed stream must not leave a stale `.part` lying around.
        let _ = std::fs::remove_file(part_path(dest));
    }
    result.map(|()| true)
}

/// Stream `reader` into `dest` via a `.part` temp + atomic rename. Split out from
/// `download` so the file plumbing is testable without a network round-trip.
fn stream_to_file(mut reader: impl Read, dest: &Path, total: Option<u64>) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
    }
    let part = part_path(dest);
    let name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("file");

    // Big files (the model) show a live byte counter; tiny ones (voices) stay
    // quiet — `init` prints a per-voice line for those.
    let show = total.is_none_or(|t| t >= PROGRESS_MIN_BYTES);

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
        if show && done - drawn >= PROGRESS_STEP {
            draw_progress(name, done, total);
            drawn = done;
        }
    }
    file.sync_all().with_context(|| format!("flush {part:?}"))?;
    drop(file);
    if show {
        draw_progress(name, done, total);
        eprintln!();
    }

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
/// Matches **structurally** (see `is_koklaude_hook`), so it finds our hook in either
/// registration form regardless of which path registered it. Cascade-cleans anything
/// it empties (group → `Stop` → `hooks`), so `merge` then `remove` restores the
/// original. Returns the settings plus how many koklaude hooks were removed, so the
/// caller can report honestly instead of always claiming success.
pub fn remove_stop_hook(mut settings: Value) -> Result<(Value, usize)> {
    let Value::Object(root) = &mut settings else {
        bail!("settings is not a JSON object");
    };
    let hooks = match root.get_mut("hooks") {
        None => return Ok((settings, 0)),
        Some(Value::Object(m)) => m,
        Some(_) => bail!("`hooks` is present but is not a JSON object"),
    };
    let stop = match hooks.get_mut("Stop") {
        None => return Ok((settings, 0)),
        Some(Value::Array(a)) => a,
        Some(_) => bail!("`Stop` is present but is not a JSON array"),
    };

    let removed = strip_our_hooks(stop);
    if stop.is_empty() {
        hooks.remove("Stop");
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok((settings, removed))
}

/// Drop every koklaude hook from each Stop group; drop a group left empty. Foreign
/// group shapes are left untouched. Returns how many koklaude hooks were removed.
fn strip_our_hooks(stop: &mut Vec<Value>) -> usize {
    let mut removed = 0;
    stop.retain_mut(|group| match group.get_mut("hooks") {
        Some(Value::Array(inner)) => {
            inner.retain(|h| {
                let ours = is_koklaude_hook(h);
                removed += ours as usize;
                !ours
            });
            !inner.is_empty()
        }
        _ => true,
    });
    removed
}

/// A fresh Stop group registering `exe` in exec form (no shell → spaces safe).
fn hook_group(exe: &Path) -> Value {
    json!({ "hooks": [{ "type": "command", "command": exe.to_string_lossy(), "args": ["hook"] }] })
}

/// Does this path's final component equal `koklaude`?
fn basename_is_koklaude(path: &str) -> bool {
    Path::new(path).file_name() == Some(OsStr::new("koklaude"))
}

/// Is this inner hook entry one of ours? Matched **structurally** — the `koklaude`
/// binary invoked with the `hook` subcommand — across both the exec form `init`
/// writes (`command` = the binary, `args` = `["hook"]`) and the shell form
/// (`command` = `".../koklaude hook"`), plus a corrupt mix of the two.
///
/// The executable is recognized when `koklaude` is the basename of either the whole
/// `command` (exec form, incl. a path with spaces) or its first whitespace token
/// (shell form); `hook` is recognized in `args` or in the command's tail. So it's
/// path-independent — a move/reinstall never orphans it, and a hand-edited entry is
/// still cleaned.
fn is_koklaude_hook(entry: &Value) -> bool {
    let Some(command) = entry.get("command").and_then(Value::as_str) else {
        return false;
    };
    let first_token = command.split_whitespace().next().unwrap_or(command);
    let exe_is_koklaude = basename_is_koklaude(command) || basename_is_koklaude(first_token);

    let arg_has_hook = entry
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().filter_map(Value::as_str).any(|s| s == "hook"));
    let command_tail_has_hook = command.split_whitespace().skip(1).any(|t| t == "hook");

    exe_is_koklaude && (arg_has_hook || command_tail_has_hook)
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

    let agent = http_agent();
    println!("fetching into {} (IPv4)…", cfg.home.display());

    print!("  model kokoro-v1.0.onnx (~310 MB): ");
    std::io::stdout().flush().ok();
    if !download(&agent, &model_url(), &cfg.model_path())? {
        println!("present, skipped");
    }

    let voices_dir = cfg.voices_dir();
    let n = VOICES.len();
    let mut fetched = 0usize;
    for (i, name) in VOICES.iter().enumerate() {
        print!("  [{:>2}/{n}] voice {name} … ", i + 1);
        std::io::stdout().flush().ok();
        let got = download(
            &agent,
            &voice_url(name),
            &voices_dir.join(format!("{name}.bin")),
        )?;
        fetched += got as usize;
        println!("{}", if got { "ok" } else { "present" });
    }
    info!(voices = n, fetched, "init: model + voices ready");

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
        let mut removed = 0;
        rewrite_settings(&settings, |s| {
            let (s, n) = remove_stop_hook(s)?;
            removed = n;
            Ok(s)
        })?;
        if removed > 0 {
            println!("removed koklaude Stop hook from {}", settings.display());
        } else {
            println!(
                "no koklaude Stop hook found in {} — nothing to remove",
                settings.display()
            );
        }
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
    // Re-encode from components: strips trailing slashes and `.` segments.
    let home = home.components().as_path();

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
        let dest = scratch("net").join("config.json");
        let _ = std::fs::remove_file(&dest);
        let got = download(&http_agent(), &format!("{HF_BASE}/config.json"), &dest).unwrap();
        assert!(got, "fetched (not skipped)");
        assert!(std::fs::metadata(&dest).unwrap().len() > 0);
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn download_skips_when_dest_present() {
        let dest = scratch("skip").join("present.bin");
        std::fs::write(&dest, b"x").unwrap();
        // Bogus URL: if the skip works, it's never fetched, so this can't error.
        let got = download(&http_agent(), "http://invalid.invalid/nope", &dest).unwrap();
        assert!(!got, "present → skipped, not re-fetched");
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

        rewrite_settings(&path, |s| remove_stop_hook(s).map(|(v, _)| v)).unwrap();
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
        let exe = Path::new("/opt/my tools/bin/koklaude");
        let got = merge_stop_hook(json!({}), exe).unwrap();
        let ours = our_hooks(&got);
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0]["command"], "/opt/my tools/bin/koklaude");
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
        let (got, removed) = remove_stop_hook(with).unwrap();
        assert_eq!(got, json!({})); // emptied group → Stop → hooks all pruned
        assert_eq!(removed, 1); // and it reports the one it took
    }

    #[test]
    fn remove_preserves_other_stop_groups() {
        let before = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] }
        });
        let with = merge_stop_hook(before.clone(), Path::new(EXE)).unwrap();
        let (got, _) = remove_stop_hook(with).unwrap();
        assert_eq!(got, before); // user's hook survives untouched
    }

    /// #3 regression: remove finds our hook structurally, so a hook registered
    /// from one path is removed even though no path/command is passed in.
    #[test]
    fn remove_is_orphan_proof_across_paths() {
        let with = merge_stop_hook(json!({}), Path::new("/some/old/koklaude")).unwrap();
        assert_eq!(remove_stop_hook(with).unwrap(), (json!({}), 1));
    }

    #[test]
    fn remove_is_noop_when_absent() {
        let settings = json!({ "hooks": { "PreToolUse": [] } });
        assert_eq!(remove_stop_hook(settings.clone()).unwrap(), (settings, 0));
        assert_eq!(remove_stop_hook(json!({})).unwrap(), (json!({}), 0));
    }

    #[test]
    fn merge_then_remove_round_trips() {
        for original in [
            json!({}),
            json!({ "model": "opus" }),
            json!({ "hooks": { "PreToolUse": [{ "hooks": [{ "type": "command", "command": "x.sh" }] }] } }),
            json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "mine.sh" }] }] } }),
        ] {
            let (round, _) =
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

    /// Wrap an inner hook entry in a full settings doc so `remove_stop_hook` can
    /// be run against it.
    fn settings_with(entry: Value) -> Value {
        json!({ "hooks": { "Stop": [{ "hooks": [entry] }] } })
    }

    /// Robust matcher: a hook registered in **shell form** (no `args`, the whole
    /// invocation in `command`) is recognized and removed — not just exec form.
    #[test]
    fn remove_finds_shell_form_hook() {
        let shell = settings_with(json!({
            "type": "command", "command": "/usr/local/bin/koklaude hook"
        }));
        let (got, removed) = remove_stop_hook(shell).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(got, json!({}));
    }

    /// Robust matcher: the corrupt mix (`command` carries a trailing ` hook` *and*
    /// `args` is present) — the state a botched hand-edit produced — is still ours,
    /// so `uninstall`/`init` can clean it instead of orphaning it.
    #[test]
    fn remove_finds_corrupt_mixed_form_hook() {
        let corrupt = settings_with(json!({
            "type": "command", "command": "/usr/local/bin/koklaude hook", "args": ["hook"]
        }));
        let (got, removed) = remove_stop_hook(corrupt).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(got, json!({}));
    }

    /// A foreign Stop hook that merely *mentions* koklaude-ish words must NOT match:
    /// wrong basename, or koklaude without the `hook` subcommand.
    #[test]
    fn remove_leaves_foreign_and_non_hook_koklaude_invocations() {
        for entry in [
            json!({ "type": "command", "command": "/usr/bin/koklaude-wrapper", "args": ["hook"] }),
            json!({ "type": "command", "command": "/usr/local/bin/koklaude", "args": ["on"] }),
            json!({ "type": "command", "command": "/usr/local/bin/koklaude on" }),
        ] {
            let before = settings_with(entry);
            let (got, removed) = remove_stop_hook(before.clone()).unwrap();
            assert_eq!(removed, 0, "must not claim {before}");
            assert_eq!(got, before, "must leave {before} untouched");
        }
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
        // Final component is a home dir / a parent — never koklaude → refused.
        // (These bail at the name check, so the paths need not exist on disk.)
        assert!(purge_home(Path::new("/home/someone")).is_err());
        assert!(purge_home(Path::new("/")).is_err());
        assert!(purge_home(Path::new("/home/someone/.config")).is_err());
    }

    #[test]
    fn purge_is_noop_when_absent() {
        let home = scratch("purge-absent").join("koklaude");
        let _ = std::fs::remove_dir_all(&home);
        purge_home(&home).unwrap(); // nothing to remove → Ok
    }

    /// Even if a symlink is *named* `koklaude`, we must not follow it and nuke
    /// its target — with OR without a trailing slash (a trailing slash makes
    /// `lstat` dereference the symlink, which must not defeat the guard).
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

        let with_slash = PathBuf::from(format!("{}/", link.display()));
        for p in [&link, &with_slash] {
            assert!(purge_home(p).is_err(), "must refuse symlink: {p:?}");
            assert!(
                target.join("keep.txt").exists(),
                "symlink target must survive ({p:?})"
            );
        }
    }
}
