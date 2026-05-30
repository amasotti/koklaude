# koklaude

Local, offline TTS for Claude Code. Two-crate Rust workspace:
- `crates/hanasu` — the TTS engine (Kokoro-82M via `ort`/ONNX + espeak g2p).
  Assistant-agnostic, later extractable. MUST NOT import Claude-Code-specific code.
- `crates/koklaude` — the binary: CLI, Stop-hook, daemon, setup. All
  Claude-Code-specific code lives here only.

## Working with me (hard rules)
- **Never commit, never delete files.** Toni does these. Offer, don't execute.
- **Small, reviewable slices.** Incremental: build → review → commit → refactor →
  test, with Toni in the loop. No overstepping, no over-engineering (KISS/YAGNI).
- When asked for X, do X — don't run ahead into implementation.
- **Clippy + tests every turn.** Keep clippy clean and tests green as we go, not
  at the end.
- **Solo project** (until stated otherwise): private repo, used only by Toni. No
  contributor guidelines, no external-contributor concerns, no backward-compat
  burden — don't add them. (Reinforces: don't over-engineer.)

## Commands (via `just`)
- `just check` — `cargo check --workspace`
- `just clippy` — clippy, warnings = errors
- `just test` — `cargo test --workspace`
- `just fmt` — format
- `just spike [voice] [text]` — Phase 1 engine spike → synth + play WAV

## Conventions
- Rust edition 2024; **MIT** (espeak-ng invoked as an external CLI, not linked —
  docs/decisions.md D3).
- clippy clean (`-D warnings`) + `cargo fmt` before calling work done.
- Unit-test pure logic (tokenization, text cleaning); smoke-test synth.
- Claude works closely following the plan, brainstorming what is clear or undecided there, after approval implementing it. Phases that are completed are also marked as completed in the plan.md

## Prerequisites (manual today; `koklaude init` automates later — Phase 5)
- `espeak-ng` on PATH; model + voices under `~/.claude/koklaude/`.
  See docs/prerequisites.md.
- `*.onnx`, `*.bin`, `*.wav` are gitignored (large / scratch — never commit).

## Status & docs
- Phases: docs/plan.md (Phase 1 ✅ engine spike; Phase 2 🎯 engine API).
- Architecture: docs/architecture.md · Decisions/ADRs: docs/decisions.md ·
  Spike repro: docs/spike.md · Prereqs: docs/prerequisites.md.
- Verified Kokoro ONNX contract: inputs `tokens` i64[1,seq], `style` f32[1,256],
  `speed` f32[1]; output `audio` f32 (mono 24 kHz). Full detail: docs/plan.md.

## Gotchas
- g2p shells out to the `espeak-ng` CLI (not linked) — keeps the project MIT
  (decisions.md D3/D4).
- `crates/hanasu/examples/spike.rs` is throwaway; its naive tokenizer is NOT the
  real one (Phase 2 builds that).
- Python stdlib `wave` can't parse Kokoro's f32 (IEEE-float) WAV; `afplay` does.
