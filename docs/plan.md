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
char-map drops punctuation — Phase 2 must preserve punctuation (see its spec).

## Phase 2 — `hanasu` engine API ✅
Public API `Engine::load(model, voices, voice, speed)` → `synth(text) -> Audio
{ samples, sample_rate }` (pure samples; the binary owns WAV + `afplay`).
Pipeline: text → espeak CLI g2p **preserving punctuation** → vocab tokenize
(clamp ≤ 510) → `ort` inference → samples (kokoro-onnx reference, not full Misaki).
Modules: `error` (thiserror) · `voice` (npz via `zip`) · `g2p` (espeak CLI) ·
`tokenizer` (split/interleave/encode) · `engine` (`Session` in a `Mutex`). 17 tests
incl. an end-to-end smoke test. Full spec: [`phase2-engine-api.md`](phase2-engine-api.md).

## Phase 3 — `koklaude` front end (pure, testable) 🎯 next
- `transcript`: parse Stop-hook stdin JSON + extract the last assistant turn.
- `clean`: markdown → speakable prose (drop code, strip markdown) — rebuilt with
  unit tests.
- `config` + toggle flag (`on`/`off`).
- No daemon yet: `koklaude say "..."` can synth+play directly to validate the chain.

### Slices (working notes — iterate, then delete on phase completion)
Each slice = one reviewable PR-sized step: build → clippy/tests green → review.
Pure logic before I/O; the chain-validating `say` lands early so we feel sound.

- **3a — `config` (paths + defaults).** Pure module: resolve the koklaude home
  (`~/.claude/koklaude/`, env-overridable for tests), locate `model.onnx` +
  `voices.bin`, hold default voice + speed. No file format yet (just paths +
  a `Config` struct with defaults). Unit-test path resolution.
- **3b — `say` end-to-end.** Wire `koklaude say "..."` → `config` paths →
  `hanasu::Engine::load` → `synth` → write temp WAV → `afplay`. Proves the
  binary↔engine chain with zero daemon. Smoke-test (gated on model presence,
  like hanasu's e2e test).
- **3c — `clean` (markdown → speakable prose).** Pure fn `clean(&str) -> String`:
  drop fenced code blocks + inline code, strip markdown markup (headings, lists,
  emphasis, links→text), collapse whitespace. Heavily unit-tested (this is the
  quality-of-speech core). Not yet wired into anything.
- **3d — `transcript` (hook input → last assistant turn).** Pure: (1) parse the
  Stop-hook stdin JSON (`serde`) to get `transcript_path`; (2) read that JSONL
  and extract the text of the last assistant turn. Fixture-driven tests (commit a
  small sample transcript). Returns plain text — `clean` is applied by the caller.
- **3e — `on`/`off` toggle.** Enabled-flag as a file under the koklaude home
  (presence = on). Pure `is_enabled()` + `enable()`/`disable()`; wire `on`/`off`
  commands. Unit-test the flag round-trip.
- **3f — configurable voice + speed.** `~/.claude/koklaude/config.toml`
  (`voice`, `speed`); `Config::load()` reads it if present, else built-in
  defaults (`toml`/`serde` already deps). `say --voice/--speed` flags override
  per-call. Precedence: CLI flag > config.toml > default. Phase 5 `init` *writes*
  this file (install params); Phase 3 only *reads* it. Tests: parse + precedence.

Open within the phase:
- Does `say` run text through `clean`, or speak raw? (Lean: raw — `say` is a
  manual test path; `hook` is what cleans.)
- `config` file format/persistence — defer to Phase 5 `init`? (Lean: yes; 3a is
  paths + in-code defaults only.)
- Exact shape of a transcript JSONL line — confirm against a real capture before
  3d (don't guess the schema).

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
  (successor to the dead `kokoroxide`; MIT).
- **More assistants:** Codex / pi adapters — a new front end per assistant, the
  same engine (see architecture › Extensibility).
- **Optional pure-Rust G2P:** `misaki-rs` with its `espeak` feature is a full
  Misaki port (POS-aware, number expansion) that could improve prosody, but it
  re-links espeak. The espeak-free build was tested and rejected (Phase 1.5: it
  spells jargon letter-by-letter). espeak CLI stays the default.
- Linux/Windows audio playback (beyond macOS `afplay`).

## Open questions (revisit as we go)
- ~~Exact Kokoro ONNX I/O contract~~ — ✅ resolved in Phase 1 (see above).
- Best default voice.
- Streaming/chunked synthesis for long replies vs. synth-then-play.
- crates.io name for `hanasu` at publish time (the name is free today).
