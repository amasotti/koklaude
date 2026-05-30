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

// Unused until the hook (Phase 4) wires it — remove then.
#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Stop-hook stdin payload (only the field we need).
#[derive(Debug, Deserialize)]
struct HookInput {
    transcript_path: PathBuf,
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
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

/// Parse the Stop-hook stdin JSON and return the transcript path.
pub fn transcript_path_from_hook(stdin: &str) -> Result<PathBuf> {
    let input: HookInput = serde_json::from_str(stdin).context("parse Stop-hook input JSON")?;
    Ok(input.transcript_path)
}

/// Extract the last assistant turn's spoken text from a transcript JSONL.
/// `None` if the turn has no speakable text (e.g. ended on a tool call).
pub fn last_assistant_turn(jsonl: &str) -> Option<String> {
    let entries: Vec<Entry> = jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        // Tolerate lines whose shape we don't model (titles, snapshots, ...).
        .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
        .collect();

    // Last real user prompt = the turn boundary. None → whole transcript.
    let start = entries
        .iter()
        .rposition(is_user_prompt)
        .map_or(0, |i| i + 1);

    let mut texts = Vec::new();
    for entry in &entries[start..] {
        if entry.kind != "assistant" {
            continue;
        }
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

    if texts.is_empty() {
        return None;
    }
    Some(texts.join("\n\n"))
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
    fn reads_transcript_path() {
        let stdin =
            r#"{"session_id":"abc","transcript_path":"/tmp/x.jsonl","hook_event_name":"Stop"}"#;
        assert_eq!(
            transcript_path_from_hook(stdin).unwrap(),
            PathBuf::from("/tmp/x.jsonl")
        );
    }

    #[test]
    fn missing_transcript_path_errors() {
        assert!(transcript_path_from_hook(r#"{"session_id":"abc"}"#).is_err());
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
            last_assistant_turn(SAMPLE).as_deref(),
            Some("Let me check.\n\nHere is the answer.")
        );
    }

    #[test]
    fn ignores_unmodelled_lines() {
        let jsonl = "{\"type\":\"ai-title\",\"title\":\"x\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"q\"}}\n{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Hi.\"}]}}";
        assert_eq!(last_assistant_turn(jsonl).as_deref(), Some("Hi."));
    }

    #[test]
    fn turn_ending_on_tool_use_has_no_text() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"go"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}}"#;
        assert_eq!(last_assistant_turn(jsonl), None);
    }

    #[test]
    fn empty_transcript_is_none() {
        assert_eq!(last_assistant_turn(""), None);
    }
}
