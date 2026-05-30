# Plan

How we get from scaffold to a tool that speaks. Built **iteratively and together**
— small, reviewable steps, tests before logic where it matters. Each phase ends in
something we can run and check.

## Phase 0 — Scaffold ✅
- Cargo workspace: `hanasu` (engine lib) + `koklaude` (binary).
- Docs: README, architecture, decisions, this plan.
- CLI skeleton (`init`/`daemon`/`hook`/`on`/`off`/`say`) compiles.
- `cargo check --workspace` is green.

## Phase 1 — Engine spike (de-risk the one real unknown) ✅
**Goal:** prove text → audible WAV through `hanasu`, end to end, once.
**Done:** `crates/hanasu/examples/spike.rs` runs the full chain (espeak g2p →
vocab tokenize → `ort` inference → WAV) and plays clear audio; voice switching
across all 54 voices works. Reproduction: `docs/spike.md`; prereqs:
`docs/prerequisites.md`.

**Verified ONNX I/O contract** (the unknown this phase existed to pin):
- inputs: `tokens` int64 `[1, seq]` (**not** `input_ids`), `style` f32 `[1, 256]`,
  `speed` f32 `[1]`
- output: `audio` f32 `[audio_length]` — mono PCM @ 24 kHz
- tokens wrapped with a leading/trailing `0`; style row selected by token count
- vocab = hexgrad/Kokoro-82M `config.json` › `vocab` (114 entries)

**Surfaced for Phase 2:** espeak emits newlines on clause breaks and the naive
char-map drops punctuation — real Misaki normalization is needed.

## Phase 2 — `hanasu` engine API 🎯 next
- Add deps: `ort`, `espeak-rs` (espeak-ng bindings), audio/WAV.
- Public API: load the model + a voice once; `synth(text) -> wav`.
- Pipeline: text → `espeak-ng` IPA phonemes → tokenize (Misaki vocab) → `ort` inference → samples.
- Unit tests for tokenization + the phoneme mapping; a smoke test for `synth`.

## Phase 3 — `koklaude` front end (pure, testable)
- `transcript`: parse Stop-hook stdin JSON + extract the last assistant turn.
- `clean`: markdown → speakable prose (drop code, strip markdown) — rebuilt with
  unit tests.
- `config` + toggle flag (`on`/`off`).
- No daemon yet: `koklaude say "..."` can synth+play directly to validate the chain.

## Phase 4 — Daemon + hook
- `daemon`: warm engine, unix socket, serial playback **queue**, 30-min idle exit.
- `client`: connect (spawn daemon if absent), send text, never block Claude Code.
- Wire `koklaude hook`: transcript → clean → daemon.
- Integration test: fixture transcript → non-empty audio.

## Phase 5 — Setup / one-command install
- `koklaude init`: download model + voices to `~/.claude/koklaude/`, write default
  config, **merge** the Stop hook into `~/.claude/settings.json` (preserving
  existing hooks), enable.
- Detect `espeak-ng`; if missing, print the install hint (`brew install espeak-ng`).

## Phase 6 — Polish & ship
- Release binaries; make the README "Install & use" real.
- Voice + speed config; pick a good default voice.
- Short demo (asciinema / audio clip).

## Later (post-1.0)
- **Extract `hanasu`** to its own repo (`git subtree split`) and publish to
  crates.io — the maintained Kokoro engine on `ort` 2.0 the ecosystem is missing
  (successor to the dead `kokoroxide`; GPL-3.0).
- **More assistants:** Codex / pi adapters — a new front end per assistant, the
  same engine (see architecture › Extensibility).
- **Optional pure-Rust G2P:** explore `misaki-rs` behind a non-default feature for
  an espeak-free (and thus MIT-licensable) build — accepting weaker pronunciation
  on out-of-vocabulary and non-English text. Not a priority; espeak is the default.
- Linux/Windows audio playback (beyond macOS `afplay`).

## Open questions (revisit as we go)
- ~~Exact Kokoro ONNX I/O contract~~ — ✅ resolved in Phase 1 (see above).
- Best default voice.
- Streaming/chunked synthesis for long replies vs. synth-then-play.
- crates.io name for `hanasu` at publish time (the name is free today).
