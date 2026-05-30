//! `koklaude hook`: the Stop-hook entrypoint. Reads the hook JSON on stdin,
//! pulls the last assistant turn, cleans it to speakable prose, and ships it to
//! the daemon.
//!
//! Failure policy: **never block or fail Claude Code**. Every error — disabled,
//! malformed payload, missing model, daemon unreachable — is logged to stderr
//! and the hook still exits 0. Worst case is silence, never a stuck assistant.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::{clean, client, toggle, transcript};

/// Entry point — always returns `Ok` so Claude Code never sees a failure.
pub fn run() -> Result<()> {
    if let Err(e) = speak_reply() {
        eprintln!("koklaude hook: {e:#}");
    }
    Ok(())
}

/// Read stdin, resolve the reply, hand it to the daemon. Errors bubble to `run`,
/// which logs and swallows them.
fn speak_reply() -> Result<()> {
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("read hook stdin")?;

    let cfg = Config::load()?;
    let enabled = toggle::is_enabled(&cfg.home);

    let Some(text) = reply_to_speak(&stdin, enabled, |p| std::fs::read_to_string(p))? else {
        return Ok(()); // disabled, or nothing worth speaking
    };
    client::send(&cfg.socket_path(), &text)
}

/// Pure pipeline (but for the injected reader): hook stdin → cleaned reply.
/// `Ok(None)` when there's nothing to speak (disabled, no text turn, empty after
/// cleaning); `Err` on a malformed payload or unreadable transcript.
fn reply_to_speak(
    stdin: &str,
    enabled: bool,
    read: impl FnOnce(&Path) -> std::io::Result<String>,
) -> Result<Option<String>> {
    if !enabled {
        return Ok(None);
    }
    let path = transcript::transcript_path_from_hook(stdin)?;
    let jsonl = read(&path).with_context(|| format!("read transcript {path:?}"))?;
    let Some(raw) = transcript::last_assistant_turn(&jsonl) else {
        return Ok(None);
    };
    let spoken = clean::clean(&raw);
    if spoken.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(spoken))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// User prompt + an assistant turn whose markdown needs cleaning.
    const TRANSCRIPT: &str = r##"{"type":"user","message":{"role":"user","content":"hi"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"# Title\n\nHello **world**."}]}}"##;

    fn hook_stdin(path: &str) -> String {
        format!(r#"{{"transcript_path":"{path}","hook_event_name":"Stop"}}"#)
    }

    #[test]
    fn disabled_says_nothing_and_never_reads() {
        let out = reply_to_speak(&hook_stdin("/x.jsonl"), false, |_| {
            panic!("must not touch the transcript when disabled")
        })
        .unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn cleans_last_turn_when_enabled() {
        let out = reply_to_speak(&hook_stdin("/whatever.jsonl"), true, |_| {
            Ok(TRANSCRIPT.to_string())
        })
        .unwrap();
        assert_eq!(out.as_deref(), Some("Title\nHello world."));
    }

    #[test]
    fn turn_with_no_text_is_none() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"go"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}"#;
        let out = reply_to_speak(&hook_stdin("/x"), true, |_| Ok(jsonl.to_string())).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn malformed_stdin_errors() {
        let out = reply_to_speak("not json at all", true, |_| Ok(String::new()));
        assert!(out.is_err());
    }

    #[test]
    fn unreadable_transcript_errors() {
        let out = reply_to_speak(&hook_stdin("/missing"), true, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        });
        assert!(out.is_err());
    }
}
