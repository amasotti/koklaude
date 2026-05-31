# Debugging: koklaude hook doesn't speak

Procedure to find why an assistant Stop hook stays silent while `koklaude say` works.
Run each step in your own shell (`! <cmd>` inside Claude Code, or a terminal).

## What's already ruled out

- **Engine + playback.** `koklaude say "test"` is audible → synth, model, voice,
  and `afplay` all work. `say` does **standalone** playback and does **not** go
  through the daemon.

So the failure is isolated to the path the hook uses and `say` does not: enabled?
→ transcript parse → unix socket → **daemon** synth + play.

## Step 1 — Is speech on?

```sh
ls ~/.config/koklaude/enabled   # present = on; absent = muted
koklaude on                     # (re-)enable if missing
```

A muted hook exits early and logs nothing — the most common "silence".

## Step 2 — Read the log (the primary tool)

Every hook fire, lifecycle action, and daemon synth/playback error is written to
a daily JSON log under `~/.koklaude/logs/` (see [`logging.md`](logging.md)) — the
daemon's stderr goes to `/dev/null`, so this file is where its errors surface.

```sh
tail -n 20 ~/.koklaude/logs/koklaude.$(date +%F).jsonl | jq .
```

Read it by `target`:

- **`koklaude::hook` — `hook fired`** with `"outcome":"None"` → nothing to speak.
  Check `dropped`/`retries`: a turn that ended on a tool call is *correctly*
  silent; persistent `retries` with `dropped > 0` means the transcript read kept
  racing a partial flush.
- **No `hook fired` line at all** → the hook never ran (hook not registered, or
  muted — Step 1). Confirm registration:
  `jq '.hooks.Stop' ~/.claude/settings.json` for Claude Code, or
  `jq '.hooks.Stop' ~/.codex/hooks.json` for Codex.
- **`koklaude::daemon` — `synth failed` / `playback failed`** → that's the root
  cause; the `error` field has the detail.
- **`daemon started` then nothing** → the request never reached it (socket /
  client issue); continue to Step 3.

## Step 3 — Watch the daemon live (only if the log is inconclusive)

For ONNX-runtime-level detail the filtered log omits, run a daemon in the
foreground so its full stderr is visible:

```sh
pkill -f 'koklaude daemon'
rm -f ~/.config/koklaude/daemon.sock
koklaude daemon                       # leave running in this terminal
```

Then, in another terminal, fire the hook at a real transcript:

```sh
T=$(ls -t ~/.claude/projects/*/*.jsonl | head -1)
printf '{"transcript_path":"%s","session_id":"debug","hook_event_name":"Stop"}' "$T" \
  | koklaude hook
```

For Codex, test the primary hook payload path:

```sh
printf '{"hook_event_name":"Stop","session_id":"debug","turn_id":"debug","last_assistant_message":"hello"}' \
  | koklaude codex-hook
```

- **Audio + no error** → the path works; the original miss was transient (cold
  start, or a tool-call turn = correctly silent).
- **Error in the daemon terminal** → root cause; read it and fix.
- **Hook prints a `koklaude hook:` error** → client-side (transcript parse /
  socket connect), not the daemon.

## Step 4 — Stale installed binary

If the binary on PATH lags the source tree, its daemon/socket protocol may not
match. Rebuild and reinstall, which also re-registers the Stop hook:

```sh
koklaude --version                              # installed
rg '^version' Cargo.toml                         # source workspace
cargo install --path crates/koklaude --force     # rebuild + reinstall
koklaude init                                    # re-register the Stop hook
```
