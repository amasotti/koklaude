//! `koklaude hook`: the Stop-hook entrypoint. Reads the hook JSON on stdin,
//! pulls the last assistant turn, cleans it to speakable prose, and ships it to
//! the daemon.
//!
//! Failure policy: **never block or fail Claude Code**. Every error — malformed
//! payload, missing model, daemon unreachable — is logged to stderr and the
//! hook still exits 0. Worst case is silence, never a stuck assistant.

use std::io::Read;
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::{clean, client, toggle, transcript};

/// The terminal CLI can fire Stop before flushing the final assistant line,
/// leaving the transcript's last line as partial JSON. Re-read with backoff a
/// few times before trusting it — mirrors `client.rs` but smaller (the file is
/// already on disk, only the tail is in flight). Bounded so a genuinely broken
/// transcript can't stall the hook.
const READ_RETRIES: u32 = 5;
const READ_INTERVAL: Duration = Duration::from_millis(100);

/// Entry point — always returns `Ok` so Claude Code never sees a failure.
pub fn run() -> Result<()> {
    if let Err(e) = speak_reply() {
        eprintln!("koklaude hook: {e:#}");
    }
    Ok(())
}

/// Resolve the last reply and hand it to the daemon. Errors bubble to `run`,
/// which logs and swallows them. Short-circuits before any work when muted.
fn speak_reply() -> Result<()> {
    let cfg = Config::load()?;
    if !toggle::is_enabled(&cfg.home) {
        return Ok(());
    }

    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("read hook stdin")?;
    let path = transcript::transcript_path_from_hook(&stdin)?;

    let Some(text) = reply_to_speak(&path)? else {
        return Ok(()); // nothing worth speaking
    };
    client::send(&cfg.socket_path(), &text)
}

/// Read the transcript and return the cleaned text of the last assistant turn,
/// or `None` when there's nothing to speak (no text turn, or empty after
/// cleaning). `Err` only on an unreadable transcript.
///
/// If the final line is partial JSON (CLI raced the flush) we re-read with
/// backoff before giving up; a clean tool-only turn never retries.
fn reply_to_speak(path: &Path) -> Result<Option<String>> {
    let mut turn = read_turn(path)?;
    let mut attempts = 0;
    while turn.last_line_partial && attempts < READ_RETRIES {
        thread::sleep(READ_INTERVAL);
        attempts += 1;
        turn = read_turn(path)?;
    }

    // Diagnosable event (until the #2 log module lands, stderr is the only sink).
    if turn.dropped > 0 || attempts > 0 {
        eprintln!(
            "koklaude hook: transcript {path:?} text={:?} dropped={} retries={}",
            turn.text.as_deref().map(str::len),
            turn.dropped,
            attempts,
        );
    }

    let Some(raw) = turn.text else {
        return Ok(None);
    };
    let spoken = clean::clean(&raw);
    Ok((!spoken.trim().is_empty()).then_some(spoken))
}

/// Read and parse the transcript at `path` into a [`transcript::Turn`].
fn read_turn(path: &Path) -> Result<transcript::Turn> {
    let jsonl =
        std::fs::read_to_string(path).with_context(|| format!("read transcript {path:?}"))?;
    Ok(transcript::parse_turn(&jsonl))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// User prompt + an assistant turn whose markdown needs cleaning.
    const TRANSCRIPT: &str = r##"{"type":"user","message":{"role":"user","content":"hi"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"# Title\n\nHello **world**."}]}}"##;

    /// A transcript whose final assistant line is truncated mid-flush.
    const PARTIAL: &str = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Hel";

    /// A fresh scratch transcript seeded with `body`; returns its path.
    fn scratch_transcript(tag: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-hook-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn cleans_a_complete_turn() {
        let path = scratch_transcript("complete", TRANSCRIPT);
        let out = reply_to_speak(&path).unwrap();
        assert_eq!(out.as_deref(), Some("Title\nHello world."));
    }

    #[test]
    fn turn_with_no_text_is_none() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"go"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}"#;
        let path = scratch_transcript("no-text", jsonl);
        let out = reply_to_speak(&path).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn unreadable_transcript_errors() {
        let out = reply_to_speak(Path::new("/no/such/koklaude.jsonl"));
        assert!(out.is_err());
    }

    #[test]
    fn retries_until_transcript_is_flushed() {
        // Seeded partial; the flush lands mid-poll — proves we re-read, not just
        // try once (mirrors client.rs::retry_connect_succeeds_once_listener_appears).
        let path = scratch_transcript("flushed", PARTIAL);
        let writer = path.clone();
        let server = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            std::fs::write(&writer, TRANSCRIPT).unwrap();
        });

        let out = reply_to_speak(&path).unwrap();
        assert_eq!(out.as_deref(), Some("Title\nHello world."));
        server.join().unwrap();
    }

    #[test]
    fn gives_up_on_persistent_partial() {
        // Transcript never finishes flushing → bounded retries, then silence.
        let path = scratch_transcript("stuck", PARTIAL);
        let out = reply_to_speak(&path).unwrap();
        assert_eq!(out, None);
    }
}
