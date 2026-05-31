# koklaude

Local, offline TTS for Claude Code. Two-crate Rust workspace:
- `crates/hanasu` — the TTS engine (Kokoro-82M via `ort`/ONNX + espeak g2p).
  Assistant-agnostic, later extractable. MUST NOT import Claude-Code-specific code.
- `crates/koklaude` — the binary: CLI, Stop-hook, daemon, setup. All
  Claude-Code-specific code lives here only.

Shipped and working on macOS (`init` → daemon → Stop hook).

## Working with me (hard rules)
- **Never commit, never delete files.** The user does these. Offer, don't execute.
- **Small, reviewable slices.** When asked for X, do X — don't run ahead into
  implementation. No overstepping, no over-engineering (KISS / YAGNI).
- **Prove claims with evidence.** Don't assert an API shape, a schema, or "it works" — verify it (run the command, read the source, check the doc) and show
  the proof. Only paste commands you actually ran.

## How we work (the loop)
This is the rhythm that built every phase — keep it:
1. **Brainstorm / plan** what's unclear before touching code; get Toni's
   approval on the approach (and on slice breakdown for anything non-trivial).
2. **Implement one small slice.** Tests before logic where it matters (pure
   logic is unit-tested; synth is smoke-tested).
3. **Verify every turn:** `just clippy` clean (`-D warnings`) + `just test`
   green *as you go*, not at the end. Build a release artifact when the change
   touches packaging.
4. **Review together**, then Toni commits. Then the next slice.
5. **Keep docs honest:** a multi-step procedure (runbook, release flow, repro)
   goes in `docs/`, not buried in chat. Update the relevant doc when behaviour
   changes; don't leave "(planned)"/"(Phase N)" hedges describing shipped code.

## Quality gates (before calling work done)
- `just clippy` clean and `just test` green.
- `cargo fmt` (`just fmt`).
- New pure logic has unit tests; synth changes have a smoke test.
- Comments are terse — only the non-obvious or an ONNX/Kokoro peculiarity. Never
  re-explain the license rationale in code.

## Commands (via `just`)
- `just check` — `cargo check --workspace`
- `just clippy` — clippy, warnings = errors
- `just test` — `cargo test --workspace`
- `just fmt` — format
- `just deny` — cargo-deny (licenses / advisories / bans)
- `just spike [voice] [text]` — engine spike → synth + play WAV

## Releasing
Conventional-commit driven, fully automated: pushing to `main` opens an
`autorelease` PR (git-cliff bumps `CHANGELOG.md` + `Cargo.toml`); merging it tags
the commit and publishes a GitHub Release with the macOS binary. You never tag by
hand. Full flow + gotchas: [`docs/release.md`](docs/release.md).

## Conventions
- Rust edition 2024; **MIT** (espeak-ng invoked as an external CLI, not linked —
  decisions.md D3).
- Verified Kokoro ONNX contract: inputs `tokens` i64[1,seq], `style` f32[1,256],
  `speed` f32[1]; output `audio` f32 (mono 24 kHz). Detail: spike.md / phase2 spec.

## Prerequisites
- `espeak-ng` on PATH (you install it; `init` only prints the hint). `koklaude
  init` downloads the model + voices into `~/.config/koklaude/`. See
  docs/prerequisites.md.
- `*.onnx`, `*.bin`, `*.wav` are gitignored (large / scratch — never commit).
  `*.gif`/`*.m4a`/`*.cast` (demo artifacts) are fine to commit.

## Docs
- Architecture: docs/architecture.md · Decisions/ADRs: docs/decisions.md ·
  Daemon internals: docs/daemon-and-sockets.md · Release: docs/release.md ·
  Demo playbook: docs/demo.md · Spike repro: docs/spike.md.

## Gotchas
- g2p shells out to the `espeak-ng` CLI (not linked) — keeps the project MIT
  (decisions.md D3/D4).
- The daemon is spawned detached with stdio → `/dev/null`, so its synth/play
  errors are invisible. To debug the hook path, run `koklaude daemon` in the
  foreground (or check that a stale shell isn't masking a fresh install on PATH).
- `crates/hanasu/examples/spike.rs` is throwaway; its naive tokenizer is NOT the
  real one (the `tokenizer` module is).
- Python stdlib `wave` can't parse Kokoro's f32 (IEEE-float) WAV; `afplay` and
  `afconvert` can.
