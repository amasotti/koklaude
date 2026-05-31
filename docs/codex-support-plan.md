# Codex Support Plan

## Summary

The README claim is correct: koklaude's engine, daemon, client, queueing, config,
and playback path are already assistant-agnostic. Codex support should be adapter
work around:

- hook entrypoint
- hook setup/uninstall
- transcript parsing fallback
- docs/tests

The daemon IPC should stay unchanged: adapters produce plain text; the daemon
speaks plain text.

## Research Notes

- Codex has lifecycle hooks. Hooks are enabled by default; the canonical feature
  key is `[features].hooks`. The older `codex_hooks` key still works as a
  deprecated alias.
- Codex has a turn-scoped `Stop` hook. Its stdin includes common fields such as
  `session_id`, `transcript_path`, `cwd`, and `model`, plus Stop-specific fields
  including `turn_id`, `stop_hook_active`, and `last_assistant_message`.
- Codex `Stop` expects JSON on stdout when the hook exits `0`; plain text stdout
  is invalid for this event.
- Codex user config lives at `~/.codex/config.toml`. Project config can live at
  `<repo>/.codex/config.toml`, but project config cannot override some
  machine-local keys.
- Codex hooks can be declared in `hooks.json` next to active config layers or
  inline under `[hooks]` in `config.toml`.
- Codex docs say `transcript_path` points to a conversation transcript for
  convenience, but the transcript format is not stable. Treat transcript parsing
  as fallback, not primary integration.
- Local Codex sessions observed under `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
  are JSONL. Assistant messages appear as `response_item` entries whose payload
  is `{"type":"message","role":"assistant","content":[...]}`. Speakable text is
  in `content[]` blocks with `type = "output_text"`.

Sources:

- https://developers.openai.com/codex/hooks
- https://developers.openai.com/codex/config-basic
- https://developers.openai.com/codex/config-reference

## Architecture

Move assistant-specific code behind adapters:

```text
Claude Code  -> adapters::claude -> clean -> client -> daemon
Codex        -> adapters::codex  -> clean -> client -> daemon
```

Current Claude-specific files:

- `crates/koklaude/src/hook.rs`
- `crates/koklaude/src/transcript.rs`
- Claude settings surgery in `crates/koklaude/src/setup.rs`

Proposed structure:

```text
crates/koklaude/src/adapters/
  mod.rs
  claude.rs
  codex.rs
```

Shared adapter output:

```rust
struct ResolvedReply {
    session_id: Option<String>,
    text: Option<String>,
}
```

## CLI

Keep existing `koklaude hook` as Claude-compatible alias for backward
compatibility.

Add explicit adapter entrypoints:

```text
koklaude claude-hook
koklaude codex-hook
```

Extend setup commands:

```text
koklaude init --adapter claude
koklaude init --adapter codex
koklaude init --adapter all

koklaude uninstall --adapter claude
koklaude uninstall --adapter codex
koklaude uninstall --adapter all
```

Default can remain `claude` to avoid surprising current users. A later release
can consider `all`.

## Codex Hook Runtime

`koklaude codex-hook` should:

1. Load koklaude config.
2. Return immediately if speech is disabled.
3. Read Codex hook JSON from stdin.
4. Resolve text using this order:
   - use `last_assistant_message` when present and non-empty;
   - otherwise parse `transcript_path`;
   - otherwise speak nothing.
5. Run existing `clean::clean`.
6. Send cleaned text through existing `client::send`.
7. Log session/turn metadata when present.
8. Never fail Codex: swallow/log errors and exit `0`.
9. Print `{}` on stdout for Codex `Stop` compatibility.

Primary input shape:

```json
{
  "hook_event_name": "Stop",
  "session_id": "...",
  "turn_id": "...",
  "transcript_path": "/path/to/session.jsonl",
  "last_assistant_message": "Final assistant text",
  "cwd": "/repo",
  "model": "gpt-5.5"
}
```

## Codex Transcript Parser

Parser should be tolerant and fixture-driven.

Algorithm:

1. Read JSONL lines.
2. Drop malformed non-final lines and count them.
3. Mark final malformed line as partial.
4. Find the last entry where:
   - top-level `type == "response_item"`
   - `payload.type == "message"`
   - `payload.role == "assistant"`
5. Extract `payload.content[]` blocks where:
   - `type == "output_text"`
   - `text` is a string
6. Join text blocks with blank lines.

Ignore:

- reasoning
- encrypted reasoning content
- function calls
- custom tool calls
- tool call outputs
- web search calls
- event messages
- token counts
- commentary/tool/log noise

Do not depend on transcript format for MVP correctness; use it only when
`last_assistant_message` is absent.

## Setup And Uninstall

Add Codex setup alongside Claude setup.

Config dir resolution:

```text
$CODEX_HOME if set
else ~/.codex
```

Do not invent `CODEX_CONFIG_DIR` unless Codex documents it later.

Prefer writing `~/.codex/hooks.json` instead of editing `config.toml` inline:

- easier merge/uninstall
- less risk of corrupting existing user config
- avoids TOML table formatting churn
- matches Codex documented hook lookup

Hook shape:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "'/absolute/path/to/koklaude' codex-hook",
            "statusMessage": "koklaude speaking"
          }
        ]
      }
    ]
  }
}
```

Uninstall should remove only koklaude Codex hook entries. It must preserve:

- foreign hook groups
- foreign hook entries in same `Stop` group
- unrelated events
- existing `config.toml`
- existing `notify = [...]`

Install should be idempotent and path-independent:

- remove prior koklaude `codex-hook` entries first
- add one fresh entry for current executable path
- detect both quoted shell form and legacy exec/args form if needed

Init output should mention Codex hook trust:

```text
Codex may require you to run /hooks once and trust the koklaude Stop hook.
```

## Tests

Parser tests are the main risk reducer.

Add tests for:

- Codex hook stdin with `last_assistant_message`.
- Codex hook stdin without `last_assistant_message` but with transcript fallback.
- Missing message produces no speech.
- Transcript extracts final assistant `output_text`.
- Transcript ignores reasoning, function calls, web search, tool output, and log
  entries.
- Transcript chooses last assistant message, not older assistant message.
- Partial final JSON line is detected.
- Codex `hooks.json` merge into absent file.
- Codex `hooks.json` merge preserves foreign hooks.
- Codex setup is idempotent.
- Codex uninstall removes only koklaude hook.
- Codex uninstall cleans empty groups/events where appropriate.

Fixtures:

```text
crates/koklaude/tests/fixtures/codex/
  plain_reply.jsonl
  tool_noise_reply.jsonl
  partial_tail.jsonl
```

Use sanitized real Codex sessions for fixtures. Keep them small and remove
private paths/content.

Quality gates:

```text
cargo test
cargo clippy
```

## Implementation Slices

### Slice 1: Adapter Boundary

- Create `adapters` module.
- Move Claude transcript parsing behind `adapters::claude`.
- Keep `koklaude hook` behavior unchanged.
- Run existing tests.

### Slice 2: Codex Hook MVP

- Add `koklaude codex-hook`.
- Parse Codex Stop stdin.
- Speak `last_assistant_message`.
- Print `{}` on stdout.
- Add unit tests for stdin parsing and no-fail behavior.

### Slice 3: Codex Setup

- Add `--adapter` arg to `init` and `uninstall`.
- Add `~/.codex/hooks.json` merge/remove.
- Preserve current default Claude behavior.
- Add setup/uninstall tests.

### Slice 4: Transcript Fallback

- Add Codex JSONL parser.
- Add fixture tests.
- Wire fallback when `last_assistant_message` is missing.

### Slice 5: Docs

- Update README:
  - “Claude Code” status remains.
  - Add “Codex support” section.
  - Mention `/hooks` trust step.
- Update architecture doc with adapter diagram.
- Update debug doc with Codex-specific checks.

## Open Questions

- Should `koklaude init --adapter all` become default once Codex support is
  stable?
- Should setup use user-level `~/.codex/hooks.json` only, or offer
  repo-local `.codex/hooks.json` for per-project installs?
- Should hook trust be documented only, or should init detect untrusted hooks and
  print a stronger warning?
- Should the binary eventually rename `koklaude` to something assistant-neutral,
  or keep brand/name stable?

## Recommendation

Build Codex support through the native Codex `Stop` hook and prefer
`last_assistant_message` over transcript parsing. Keep transcript parsing as a
fallback with real-session fixtures. Do not change daemon IPC.

