# Logging

Why koklaude keeps a persistent log, and how it's wired.

## The problem

The failure policy is "log to stderr, exit 0" ([`hook.rs`](../crates/koklaude/src/hook.rs)),
and the detached daemon's stderr is redirected to `/dev/null`
([`client.rs`](../crates/koklaude/src/client.rs) — `spawn_daemon`). So every
daemon synth/playback error, and every hook outcome, was **invisible**.

## The shape: one daily JSON stream

All processes — `hook`, `daemon`, CLI — write to a single daily-rotated file:

```
~/.koklaude/logs/koklaude.YYYY-MM-DD.jsonl
```

One JSON object per line. The shape (verified — `tracing-subscriber` JSON
formatter):

```json
{"timestamp":"2026-05-31T08:47:20.264000Z","level":"INFO","fields":{"message":"spoke","chars":214,"synth_ms":92,"play_ms":1840},"target":"koklaude::daemon"}
```

- `timestamp`, `level` — from `tracing`
- `target` — the emitting module path (`koklaude::daemon`, `koklaude::hook`,
  `koklaude::toggle`, …). This **is** the component: free, automatic, and
  thread-correct (a process-wide span would not follow the daemon's worker
  thread, where `spoke` and the synth/playback errors fire).
- `fields.message` + event fields
- `span.session_id` — present on hook events, which run inside a `session_id`
  span (the Stop-hook stdin carries it; it is also the transcript filename
  stem). Absent on daemon/CLI lines.

Filter one session, or just the daemon:

```sh
jq 'select(.span.session_id=="8e8c…")'      ~/.koklaude/logs/koklaude.2026-05-31.jsonl
jq 'select(.target=="koklaude::daemon")'    ~/.koklaude/logs/koklaude.2026-05-31.jsonl
```

### Why one stream and not per-session files

The daemon owns the events worth logging most — synth timing, synth/playback
errors — but it is a **session-blind singleton**: the hook sends only the reply
text over the socket ([`ipc.rs`](../crates/koklaude/src/ipc.rs)), never the
session id. Attributing daemon events to a session needs `session_id` on the
wire, which is exactly the protocol redesign scheduled for v0.2 #4. Rather than
add it twice, the daemon logs `component=daemon` lines now and gains
`session_id` when #4 lands. A single stream keeps cross-process correlation to
one `tail -f` in the meantime.

### Location

`~/.koklaude/`, **not** under the config home (`~/.config/koklaude`). Logs are
runtime output, not configuration. Overridable via `KOKLAUDE_LOG_DIR` (tests),
mirroring `config`'s `KOKLAUDE_HOME`.

## Wiring

- A `logging` module ([`logging.rs`](../crates/koklaude/src/logging.rs)) builds a
  `tracing` JSON subscriber over a `tracing-appender` daily rolling file.
- `main` calls `logging::init()` once, after arg parsing, before dispatch.
- The component is read from each event's `target` (module path) — no span
  needed, so it works across the daemon's worker thread.
- The hook enters a `session_id` span around its work, so `hook fired` and any
  downstream event carry the session.
- Call sites use plain `tracing` macros (`info!`, `error!`). The previous
  `eprintln!`s in `daemon.rs` / `hook.rs` are replaced by these.

`tracing-appender`'s rolling appender is used **blocking/synchronous** — no
background worker, no flush guard — which is correct for the short-lived hook and
CLI processes (a buffered/non-blocking writer can lose the tail when the process
exits immediately).

## Failure policy

Logging never breaks koklaude. If the log dir can't be created or the subscriber
can't be set, `init` swallows the error — worst case is no logs, never a crash.
"Never block Claude Code" still holds.

## Events

Landed incrementally:

**Daemon** — `daemon started` (voice) · `daemon idle, exiting` · `spoke`
(chars, synth_ms, play_ms) · `synth failed` (error) · `playback failed` (error).

**Hook** — `hook fired` (transcript, outcome = `Some(len)` / `None`, dropped,
retries), within the `session_id` span.

**CLI lifecycle** — `on` · `off` · `init` · `uninstall`.

Deferred to v0.2 #4: queue `stop` / purge events (and `session_id` on daemon
lines, once the protocol carries it).

## Retention

None yet. Daily files accumulate under `~/.koklaude/logs/`. A size/age cap is a
later refinement (v0.2 #2 left it explicitly out of scope).
