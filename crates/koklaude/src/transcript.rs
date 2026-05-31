//! Claude Code Stop-hook input → the text of the last assistant turn.
//!
//! The Stop hook receives a small JSON on stdin pointing at a transcript file;
//! that file is JSONL, one entry per line. We extract what's worth speaking:
//! the `text` blocks of the assistant's final turn (skipping `thinking` and
//! `tool_use`). Returns raw markdown — the caller runs it through `clean`.
//!
//! Turn boundary: a real user prompt has **string** content; tool results are
//! *also* `type: "user"` but carry a `tool_result` array. So the last turn is
//! everything after the last string-content user line.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Stop-hook stdin payload (the fields we use).
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub transcript_path: PathBuf,
    /// Claude session id (also the transcript filename stem). Optional so a
    /// minimal/legacy payload still parses; used to tag logs.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// One transcript JSONL line (the subset we read).
#[derive(Debug, Deserialize)]
struct Entry {
    #[serde(rename = "type")]
    kind: String,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Content,
}

/// A message's content is either a plain string (real user prompt) or an array
/// of typed blocks (assistant output, tool results).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Content {
    // The payload is never read — its presence just discriminates a real user
    // prompt (string) from a tool-result (array) during untagged deserialization.
    Text(#[allow(dead_code)] String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

/// Parse the Stop-hook stdin JSON into its [`HookInput`].
pub fn parse_hook_input(stdin: &str) -> Result<HookInput> {
    serde_json::from_str(stdin).context("parse Stop-hook input JSON")
}

/// Outcome of parsing a transcript: the speakable text plus the signals that
/// tell a partial flush (CLI raced the Stop hook) apart from a clean tool-only
/// turn.
///
/// - `last_line_partial` — final non-empty line was broken JSON; the transcript
///   is still being written, retry-read before trusting it.
/// - `turn_started` — a real user-prompt boundary was found, so a response is
///   expected. When `true` and `assistant_seen` is `false`, the assistant entry
///   hasn't been flushed yet (hook fired before transcript was updated).
/// - `assistant_seen` — at least one assistant entry appeared after the turn
///   boundary. `false` for an in-progress response (race) or empty transcript.
pub struct Turn {
    pub text: Option<String>,
    pub last_line_partial: bool,
    pub dropped: usize,
    pub turn_started: bool,
    pub assistant_seen: bool,
}

/// Extract the last assistant turn's spoken text from a transcript JSONL, plus
/// the partial-flush signals (see [`Turn`]). `text` is `None` if the turn has no
/// speakable text (e.g. ended on a tool call).
pub fn parse_turn(jsonl: &str) -> Turn {
    let lines: Vec<&str> = jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    let last_line_partial = lines
        .last()
        .is_some_and(|l| serde_json::from_str::<Entry>(l).is_err());

    let mut dropped = 0;
    let entries: Vec<Entry> = lines
        .iter()
        // Tolerate lines whose shape we don't model;
        .filter_map(|l| match serde_json::from_str::<Entry>(l) {
            Ok(e) => Some(e),
            Err(_) => {
                dropped += 1;
                None
            }
        })
        .collect();

    // Last real user prompt = the turn boundary.
    let last_user = entries.iter().rposition(is_user_prompt);
    let turn_started = last_user.is_some();
    let start = last_user.map_or(0, |i| i + 1);

    let mut texts = Vec::new();
    let mut assistant_seen = false;
    for entry in &entries[start..] {
        if entry.kind != "assistant" {
            continue;
        }
        assistant_seen = true;
        if let Some(Message {
            content: Content::Blocks(blocks),
        }) = &entry.message
        {
            for b in blocks {
                if b.kind == "text"
                    && let Some(t) = &b.text
                {
                    texts.push(t.as_str());
                }
            }
        }
    }

    let text = (!texts.is_empty()).then(|| texts.join("\n\n"));
    Turn {
        text,
        last_line_partial,
        dropped,
        turn_started,
        assistant_seen,
    }
}

/// A genuine user turn (string content), not a tool-result (`type:"user"` too).
fn is_user_prompt(entry: &Entry) -> bool {
    entry.kind == "user"
        && matches!(
            entry.message,
            Some(Message {
                content: Content::Text(_)
            })
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_transcript_path_and_session_id() {
        let stdin =
            r#"{"session_id":"abc","transcript_path":"/tmp/x.jsonl","hook_event_name":"Stop"}"#;
        let input = parse_hook_input(stdin).unwrap();
        assert_eq!(input.transcript_path, PathBuf::from("/tmp/x.jsonl"));
        assert_eq!(input.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn absent_session_id_is_none() {
        let input = parse_hook_input(r#"{"transcript_path":"/tmp/x.jsonl"}"#).unwrap();
        assert_eq!(input.session_id, None);
    }

    #[test]
    fn missing_transcript_path_errors() {
        assert!(parse_hook_input(r#"{"session_id":"abc"}"#).is_err());
    }

    // Realistic JSONL: a prior turn, then user prompt, then the final turn
    // (thinking + text + tool_use + tool_result + text).
    const SAMPLE: &str = r#"
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Old answer."}]}}
{"type":"user","message":{"role":"user","content":"new question"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me check."}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read"}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"file data"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Here is the answer."}]}}
"#;

    #[test]
    fn extracts_last_turn_text_only() {
        // After the user prompt: the two text blocks, joined; thinking/tool skipped;
        // the tool_result user line does NOT reset the turn.
        assert_eq!(
            parse_turn(SAMPLE).text.as_deref(),
            Some("Let me check.\n\nHere is the answer.")
        );
    }

    #[test]
    fn ignores_unmodelled_lines() {
        let jsonl = "{\"type\":\"ai-title\",\"title\":\"x\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"q\"}}\n{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Hi.\"}]}}";
        assert_eq!(parse_turn(jsonl).text.as_deref(), Some("Hi."));
    }

    #[test]
    fn turn_ending_on_tool_use_has_no_text() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"go"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}"#;
        let turn = parse_turn(jsonl);
        assert_eq!(turn.text, None);
        assert!(turn.turn_started);
        assert!(turn.assistant_seen); // assistant IS there, just no text blocks
    }

    #[test]
    fn empty_transcript_is_none() {
        let turn = parse_turn("");
        assert_eq!(turn.text, None);
        assert!(!turn.turn_started);
        assert!(!turn.assistant_seen);
    }

    /// Race condition: hook fired before the assistant entry was written.
    /// `turn_started=true` (user prompt exists) but `assistant_seen=false`.
    /// The retry loop in `hook.rs` should re-read in this case.
    #[test]
    fn user_prompt_without_assistant_signals_race() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"what is rfc 8305?"}}"#;
        let turn = parse_turn(jsonl);
        assert_eq!(turn.text, None);
        assert!(turn.turn_started, "turn boundary found");
        assert!(
            !turn.assistant_seen,
            "no assistant entry yet — hook raced the write"
        );
    }

    #[test]
    fn fully_parsed_turn_is_not_flagged_partial() {
        let turn = parse_turn(SAMPLE);
        assert_eq!(
            turn.text.as_deref(),
            Some("Let me check.\n\nHere is the answer.")
        );
        assert!(!turn.last_line_partial);
        assert_eq!(turn.dropped, 0);
        assert!(turn.turn_started);
        assert!(turn.assistant_seen);
    }

    #[test]
    fn truncated_final_line_flags_partial() {
        // CLI fired Stop mid-flush: the final assistant line is incomplete JSON.
        let jsonl = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"q\"}}\n\
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"the real ans";
        let turn = parse_turn(jsonl);
        assert!(turn.last_line_partial);
        assert_eq!(turn.dropped, 1);
        // The broken line is dropped → no speakable text yet.
        assert_eq!(turn.text, None);
        assert!(turn.turn_started);
        assert!(!turn.assistant_seen); // broken line was dropped, so not counted
    }

    #[test]
    fn unmodelled_lines_do_not_flag_partial() {
        // A typed-but-unmodelled trailing line is valid JSON → not a partial flush.
        let jsonl = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"q\"}}\n\
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Hi.\"}]}}\n\
{\"type\":\"ai-title\",\"title\":\"x\"}";
        let turn = parse_turn(jsonl);
        assert!(!turn.last_line_partial);
        assert_eq!(turn.dropped, 0);
        assert_eq!(turn.text.as_deref(), Some("Hi."));
    }
}
