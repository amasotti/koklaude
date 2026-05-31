# Decisions

A short log of the choices behind koklaude and *why* — including the paths we
rejected, so the reasoning isn't lost. Lightweight ADR style.

## D1 — Run the model locally, never the cloud
**Decision:** Use the Kokoro-82M model on-device via ONNX. No cloud TTS.
**Why:** Safety and cost are hard requirements — code and assistant replies must
not leave the machine, and it must be free. Rules out the OpenAI-based approach
that `ybouhjira/claude-code-tts` takes.

## D2 — Build our own engine instead of using an existing crate
**Decision:** Build a thin engine (`hanasu`) on maintained primitives rather than
depend on a ready-made Kokoro crate.
**Why (verified May 2026):**
- `kokoroxide` (MIT/Apache, clean lib API) is **uninstallable** — every published
  version *and* its GitHub HEAD pin `ort = "^1.16"`, and every `ort 1.16.x` is
  yanked on crates.io. ~8 months stale, single maintainer.
- `kokorox` / `Kokoros` install (`ort 2.0-rc`) but are shaped as a CLI/server, not
  a clean embeddable library.
- No maintained, in-process Kokoro library on a current `ort` exists for Rust.

Since any usable path required real work, building our own yields a clean,
maintained foundation — `hanasu`, the in-process successor to the dead `kokoroxide`
on `ort` 2.0 — which the ecosystem is missing.

## D3 — Phonemization via `espeak-ng` (CLI)
**Decision:** Phonemize with `espeak-ng`, invoked as an **external CLI**
(`espeak-ng -q --ipa`) — not the `espeak-rs` bindings. `ort` 2.0 for ONNX inference.
**Why espeak:** Kokoro needs phonemes, and users write jargon/code/non-native
English — words a fixed dictionary lacks. A spike confirmed the pure-Rust
`misaki-rs` (espeak-free) spells such words letter-by-letter (`Kubernetes` →
"K-U-B-E-R-N-E-T-E-S"; same for `OAuth`, `stdout`, `PostgreSQL`), silently;
espeak handles arbitrary words. (`misaki-rs` *with* its espeak feature remains a
Phase 2 g2p option — better quality, but re-links espeak.)
**Why CLI, not bindings:** simpler, zero build/link risk, and it keeps the project
**license-free of GPL** — see D4.

## D4 — Cargo workspace, two crates
**Decision:** `crates/hanasu` (pure engine library) + `crates/koklaude` (the
binary: CLI, hook, daemon, setup). Daemon/socket/queue live in the **binary**, not
the engine.
**Why:** Makes the engine boundary compiler-enforced (the engine cannot import
Claude-Code code) and lets us later `git subtree split crates/hanasu` into its own
repo and publish it, with zero untangling. Kept to two crates — modules like
`transcript`/`daemon`/`config` stay inside the binary rather than becoming
micro-crates (avoiding over-engineering).
**License:** **MIT.** Because we invoke `espeak-ng` as a separate CLI process
(D3) rather than linking it, its GPL doesn't propagate — so both crates are MIT,
and `hanasu` is cleanly publishable. The user installs `espeak-ng` themselves
(docs/prerequisites.md).

## D5 — Integrate via Claude Code's Stop hook
**Decision:** A `Stop` hook runs `koklaude hook` after each assistant turn.
**Why:** It's the deterministic "assistant finished" signal. The hook reads the
session transcript for the last reply.

## D6 — Speak the full reply, with code stripped
**Decision:** Speak the whole assistant message, but strip fenced/inline code and
markdown syntax.
**Why:** Code read aloud is noise; prose is the signal. Speaking only the first
sentence (like the Go project) loses too much.

## D7 — Overlapping replies queue, never interrupt
**Decision:** If a new reply arrives while audio is playing, queue it.
**Why:** Stale audio is mildly annoying; losing half the information is worse.

## D8 — Warm daemon with idle shutdown
**Decision:** A background daemon holds the model in memory and serves the hook
over a unix socket; it auto-spawns on first use and exits after 30 min idle.
**Why:** Cold-loading the model on every reply is too slow. Idle shutdown frees
RAM when you're not coding.

## D9 — Instant toggle via a flag file
**Decision:** `~/.config/koklaude/enabled` presence = on; `koklaude on` / `off`
flip it. The hook checks it first and exits silently when off.
**Why:** Turn speech on/off mid-session with no uninstall or restart.

## D10 — `std` unix sockets, no async runtime
**Decision:** Build the daemon on `std::os::unix::net` + one worker thread + an
`mpsc` queue. No `tokio`, no async.
**Why:** There's one model and playback is serial (D7), so there is no
concurrency for an async runtime to exploit — the accept loop and a single
playback worker is the whole story. `std` keeps the dependency tree and the
mental model small (KISS). Idle shutdown is the one rough edge: the accept loop
blocks in `incoming()` and `std` can't interrupt it, so the worker calls
`std::process::exit(0)` when idle (nothing is playing then, so it's clean) rather
than threading a shutdown signal through a blocking accept.

## D11 — Socket wire protocol and lifecycle
**Decision:**
- **Framing:** one connection = one request. The client writes UTF-8 text and
  half-closes (`shutdown(Write)`); the daemon reads to **EOF**. No length prefix
  or delimiter — EOF is the boundary. Fire-and-forget, no reply.
- **Stale-socket recovery:** `std` never unlinks the socket on exit, so a kill or
  crash leaves an orphan file. On `bind` → `AddrInUse`, probe-connect: success =
  a live daemon (bail), refused = stale (unlink + rebind). The graceful idle path
  also unlinks, but the startup probe is the real safety net since signals run no
  cleanup. The client uses the same `ConnectionRefused`-means-stale signal to
  decide to respawn.
- **Detachment:** the client spawns the daemon with stdio → `/dev/null`. That
  alone keeps Claude Code from blocking (its pipe to the hook closes when the
  hook exits, independent of the long-lived daemon). **No `setsid`/double-fork** —
  verified end-to-end that the daemon survives the hook exiting on macOS.
**Why:** Each piece is the simplest thing that's actually correct; the protocol
needs no framing because there's exactly one message per connection, and the
lifecycle handling makes a killed daemon self-heal on next launch. Full walkthrough:
[`daemon-and-sockets.md`](daemon-and-sockets.md).

## Out of scope
- **Speech-to-text input.** Handled by other tools (Claude Code voice mode,
  Spokenly, Whisper). koklaude is output-only.
- **Ollama as the runtime.** It runs LLMs only (no TTS models) and isn't a general
  ONNX runtime — not usable here.
- **Building our own G2P or TTS model.** The goal is to give the assistant a voice,
  not to reinvent phonemization or train a multilingual TTS model. We wire together
  the best existing pieces (`espeak-ng` + Kokoro-82M) and stop there.
- **Bigger autoregressive models (e.g. [Chatterbox](https://github.com/resemble-ai/chatterbox)).**
  Fascinating — voice cloning, expressive tags, 23+ languages — but 4–6× larger and
  autoregressive, so slower CPU inference with variable latency. koklaude speaks short
  hook notifications, not audiobooks; Kokoro's small feed-forward graph is the right
  fit. Revisit as an optional second backend if cloning/multilingual ever matters.
