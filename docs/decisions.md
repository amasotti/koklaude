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
**Decision:** Use `espeak-ng` for grapheme→phoneme, invoked as an **external CLI**
(`espeak-ng -q --ipa`), **not** via the `espeak-rs` static-linked bindings. `ort`
2.0 (pykeio, actively maintained) for ONNX inference.
*Updated:* originally specified the `espeak-rs` bindings; switched to the CLI in
Phase 1/2 — proven in the spike, zero build/link risk, and it changes the license
footing (see Consequence).

**Why espeak at all:** Kokoro consumes phonemes, not text, so something must do
G2P, and it has to be good. The intended users write **non-native English, dense
with domain jargon and coding-assistant vocabulary** — exactly the words a fixed
embedded dictionary does *not* contain. `espeak-ng` phonemizes arbitrary words and
many languages; it's the backend Kokoro's own reference pipeline uses.

**Why not a pure-Rust dictionary G2P (`misaki-rs`) — tested, not assumed:** A Phase
1.5 spike compared `misaki-rs` (espeak-free, MIT) against the espeak CLI on a
representative workload. misaki spelled the words that matter most **letter by
letter** — `Kubernetes` → "K-U-B-E-R-N-E-T-E-S", likewise `OAuth`, `stdout`,
`PostgreSQL` — and did so *silently* (no unknown marker, so garbage would ship
unnoticed). Common words and numbers matched espeak. This confirms the original
concern: dictionary-only G2P is unusable for coding/jargon text. (`misaki-rs` *with*
its `espeak` feature is a full Misaki port and a viable **Phase 2** g2p candidate —
better quality than raw espeak — but it pulls `espeak-rs`, the binding we avoided.)

**Consequence (license — reopens the GPL choice):** espeak-ng is now a **separate
program we exec**, not a statically linked library. Arm's-length CLI invocation is
generally *not* a derivative work — which **removes the static-linking basis for
GPL-3.0** that the project assumed. So koklaude/hanasu are **not necessarily forced
to GPL**; a permissive license (MIT/Apache) may be viable while still requiring the
user to have `espeak-ng` installed (which `koklaude init` checks for / hints).
**To be decided deliberately** — this supersedes the earlier "GPL because static
link" stance. Not legal advice.

## D4 — Cargo workspace, two crates
**Decision:** `crates/hanasu` (pure engine library) + `crates/koklaude` (the
binary: CLI, hook, daemon, setup). Daemon/socket/queue live in the **binary**, not
the engine.
**Why:** Makes the engine boundary compiler-enforced (the engine cannot import
Claude-Code code) and lets us later `git subtree split crates/hanasu` into its own
repo and publish it, with zero untangling. Kept to two crates — modules like
`transcript`/`daemon`/`config` stay inside the binary rather than becoming
micro-crates (avoiding over-engineering).

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
**Decision:** `~/.claude/koklaude/enabled` presence = on; `koklaude on` / `off`
flip it. The hook checks it first and exits silently when off.
**Why:** Turn speech on/off mid-session with no uninstall or restart.

## Out of scope
- **Speech-to-text input.** Handled by other tools (Claude Code voice mode,
  Spokenly, Whisper). koklaude is output-only.
- **Ollama as the runtime.** It runs LLMs only (no TTS models) and isn't a general
  ONNX runtime — not usable here.
- **Building our own G2P or TTS model.** The goal is to give the assistant a voice,
  not to reinvent phonemization or train a multilingual TTS model. We wire together
  the best existing pieces (`espeak-ng` + Kokoro-82M) and stop there.
