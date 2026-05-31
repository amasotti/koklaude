//! `koklaude codex-hook`: Codex Stop-hook entrypoint.
//!
//! Codex requires JSON stdout for Stop hooks, so this always prints `{}` and
//! exits 0. Errors become stderr/log lines only: worst case is silence.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, info_span};

use crate::config::Config;
use crate::{clean, client, toggle};

#[derive(Debug, Deserialize)]
struct StopInput {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<PathBuf>,
    #[serde(default)]
    last_assistant_message: Option<String>,
}

pub struct CodexTranscript {
    pub text: Option<String>,
    pub dropped: usize,
    pub last_line_partial: bool,
}

/// Entry point — always prints JSON and returns `Ok` so Codex never sees a
/// failed Stop hook.
pub fn run() -> Result<()> {
    if let Err(e) = speak_reply() {
        eprintln!("koklaude codex-hook: {e:#}");
    }
    println!("{{}}");
    Ok(())
}

fn speak_reply() -> Result<()> {
    let hook_t0 = Instant::now();
    let cfg = Config::load()?;
    if !toggle::is_enabled(&cfg.home) {
        return Ok(());
    }

    let stdin_t0 = Instant::now();
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("read Codex hook stdin")?;
    let stdin_ms = stdin_t0.elapsed().as_millis() as u64;
    let input = parse_stop_input(&stdin)?;

    let span = info_span!(
        "turn",
        session_id = input.session_id.as_deref().unwrap_or("unknown"),
        turn_id = input.turn_id.as_deref().unwrap_or("unknown")
    );
    let _guard = span.enter();

    let extract_t0 = Instant::now();
    let Some(text) = resolve_reply(&input)? else {
        info!(
            stdin_ms,
            extract_ms = extract_t0.elapsed().as_millis() as u64,
            hook_ms = hook_t0.elapsed().as_millis() as u64,
            "codex hook completed without speech"
        );
        return Ok(());
    };
    let extract_ms = extract_t0.elapsed().as_millis() as u64;
    let send_t0 = Instant::now();
    client::send(&cfg.socket_path(), &text)?;
    info!(
        chars = text.chars().count(),
        stdin_ms,
        extract_ms,
        send_ms = send_t0.elapsed().as_millis() as u64,
        hook_ms = hook_t0.elapsed().as_millis() as u64,
        "codex hook completed"
    );
    Ok(())
}

fn parse_stop_input(stdin: &str) -> Result<StopInput> {
    serde_json::from_str(stdin).context("parse Codex Stop-hook input JSON")
}

fn resolve_reply(input: &StopInput) -> Result<Option<String>> {
    if let Some(text) = input
        .last_assistant_message
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        return Ok(cleaned(text));
    }

    let Some(path) = &input.transcript_path else {
        return Ok(None);
    };
    reply_from_transcript(path)
}

fn reply_from_transcript(path: &Path) -> Result<Option<String>> {
    let jsonl =
        std::fs::read_to_string(path).with_context(|| format!("read Codex transcript {path:?}"))?;
    let parsed = parse_transcript(&jsonl);
    info!(
        transcript = %path.display(),
        outcome = ?parsed.text.as_deref().map(str::len),
        dropped = parsed.dropped,
        partial = parsed.last_line_partial,
        "codex transcript parsed"
    );
    Ok(parsed.text.as_deref().and_then(cleaned))
}

fn cleaned(text: &str) -> Option<String> {
    let text = clean::clean(text);
    (!text.trim().is_empty()).then_some(text)
}

pub fn parse_transcript(jsonl: &str) -> CodexTranscript {
    let lines: Vec<&str> = jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    let last_line_partial = lines
        .last()
        .is_some_and(|l| serde_json::from_str::<Value>(l).is_err());

    let mut dropped = 0;
    let mut last_text = None;
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            dropped += 1;
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = entry.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("message")
            || payload.get("role").and_then(Value::as_str) != Some("assistant")
        {
            continue;
        }
        let texts: Vec<&str> = payload
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("output_text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect();
        if !texts.is_empty() {
            last_text = Some(texts.join("\n\n"));
        }
    }

    CodexTranscript {
        text: last_text,
        dropped,
        last_line_partial,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_last_assistant_message_first() {
        let input = parse_stop_input(
            r##"{"session_id":"s","turn_id":"t","last_assistant_message":"# Hi\n\n**There**"}"##,
        )
        .unwrap();
        assert_eq!(resolve_reply(&input).unwrap().as_deref(), Some("Hi\nThere"));
    }

    #[test]
    fn missing_message_is_silent_without_transcript() {
        let input = parse_stop_input(r#"{"session_id":"s","turn_id":"t"}"#).unwrap();
        assert_eq!(resolve_reply(&input).unwrap(), None);
    }

    #[test]
    fn transcript_extracts_last_assistant_output_text() {
        let jsonl = r#"
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"old"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"reasoning","text":"skip"},{"type":"output_text","text":"new"},{"type":"output_text","text":"answer"}]}}
"#;
        let parsed = parse_transcript(jsonl);
        assert_eq!(parsed.text.as_deref(), Some("new\n\nanswer"));
        assert_eq!(parsed.dropped, 0);
        assert!(!parsed.last_line_partial);
    }

    #[test]
    fn transcript_ignores_noise_and_detects_partial_tail() {
        let jsonl = r#"
{"type":"event_msg","msg":"skip"}
{"type":"response_item","payload":{"type":"function_call","name":"x"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"output_text","text":"skip user"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"speak"}]}}
{"type":"response_item","payload":
"#;
        let parsed = parse_transcript(jsonl);
        assert_eq!(parsed.text.as_deref(), Some("speak"));
        assert_eq!(parsed.dropped, 1);
        assert!(parsed.last_line_partial);
    }

    #[test]
    fn transcript_with_no_message_is_none() {
        let parsed =
            parse_transcript(r#"{"type":"response_item","payload":{"type":"web_search_call"}}"#);
        assert_eq!(parsed.text, None);
    }

    #[test]
    fn parses_codex_fixtures() {
        let plain = parse_transcript(include_str!("../tests/fixtures/codex/plain_reply.jsonl"));
        assert_eq!(plain.text.as_deref(), Some("Plain reply."));

        let noisy = parse_transcript(include_str!(
            "../tests/fixtures/codex/tool_noise_reply.jsonl"
        ));
        assert_eq!(noisy.text.as_deref(), Some("Final reply."));

        let partial = parse_transcript(include_str!("../tests/fixtures/codex/partial_tail.jsonl"));
        assert_eq!(partial.text.as_deref(), Some("Complete before tail."));
        assert!(partial.last_line_partial);
    }
}
