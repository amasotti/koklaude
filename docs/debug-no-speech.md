# Debugging: koklaude hook doesn't speak

Procedure to find why the Stop hook stays silent while `koklaude say` works.
Run each step in your own shell (`! <cmd>` inside Claude Code, or a terminal).

## What's already ruled out

- **Hook schema.** `{ "command": "/usr/local/bin/koklaude", "args": ["hook"] }`
  (exec form) is valid and honored — confirmed against
  https://code.claude.com/docs/en/hooks.md . `init` writes this; it is correct.
- **Engine + playback.** `koklaude say "test"` is audible → synth, model, voice,
  and `afplay` all work. `say` does **standalone** playback and does **not** go
  through the daemon.

So the failure is isolated to the path the hook uses and `say` does not: the
**daemon** (hook → unix socket → daemon synthesizes + plays). Daemon stderr is
redirected to `/dev/null` when spawned detached, so its errors are invisible.

## Step 0 — Fix the settings.json regression first

A manual edit changed the hook to shell form (`"command": ".../koklaude hook"`,
no `args`). That breaks `koklaude uninstall`, which matches our hook structurally
by basename `koklaude` **+** `args == ["hook"]`. Revert to exec form:

```jsonc
// ~/.claude/settings.json  →  hooks.Stop[].hooks[]
{ "type": "command", "command": "/usr/local/bin/koklaude", "args": ["hook"] }
```

After this, `koklaude uninstall` will recognize and remove the hook again.

## Step 1 — Confirm the installed binary is stale

```sh
/usr/local/bin/koklaude --version   # observed: 0.0.2
rg '^version' Cargo.toml            # source workspace: 0.1.0
git tag --list                      # only v0.0.1
```

The binary on PATH is behind the source tree. Its daemon/socket protocol may not
match what the current code expects. This is the prime suspect.

## Step 2 — Watch the daemon in the foreground

Stop the detached daemon (it normally auto-respawns) and run one in the
foreground so its stderr is visible:

```sh
pkill -f 'koklaude daemon'
rm -f ~/.config/koklaude/daemon.sock
/usr/local/bin/koklaude daemon          # leave running in this terminal
```

## Step 3 — Fire the hook at a real transcript, in another terminal

```sh
T=$(ls -t ~/.claude/projects/-Users-toni-halb-personal-koklaude/*.jsonl | head -1)
printf '{"transcript_path":"%s","hook_event_name":"Stop"}' "$T" \
  | /usr/local/bin/koklaude hook
```

Watch the foreground daemon terminal:
- **Audio + no error** → the path works; the original miss was transient (cold
  start, or a turn that ended on a tool call = correctly silent). Done.
- **Error in the daemon** → that's the root cause; read it and fix.
- **Hook prints a `koklaude hook:` error** → failure is client-side (transcript
  parse / socket connect), not the daemon.

## Step 4 — If the daemon path is the culprit: rebuild + reinstall

The installed 0.0.2 is stale. Rebuild from current source and reinstall, then
re-register the (correct, exec-form) hook:

```sh
just clippy && just test            # green before shipping
cargo install --path crates/koklaude --force   # or your release-binary path
koklaude init                       # re-registers the exec-form Stop hook
```

Then repeat Steps 2–3 to confirm speech.
