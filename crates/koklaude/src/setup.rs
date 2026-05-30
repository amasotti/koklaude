//! Install/uninstall surgery on Claude Code's `~/.claude/settings.json`.
//!
//! Pure JSON transforms — no filesystem here (5d owns read + atomic write). Add
//! or remove koklaude's Stop hook while preserving every other hook the user has.
//! Verified schema (code.claude.com/docs hooks): `hooks.Stop` is an array of
//! groups, each `{ "hooks": [ { "type": "command", "command": "…" } ] }`. Stop
//! has **no `matcher`** — it fires unconditionally.
// Unwired until 5d composes `init`/`uninstall`; drop this then.
#![allow(dead_code)]

use anyhow::{Result, bail};
use serde_json::{Value, json};

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

    const CMD: &str = "koklaude hook";

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
