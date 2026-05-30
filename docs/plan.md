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

## Phase 3 — `koklaude` front end (pure, testable) ✅
The Claude-Code-specific front end as small, tested modules in the `koklaude`
binary; `koklaude say "..."` validates text → speech end to end with no daemon.
Modules: `config` (home under `~/.config/koklaude`, env override `KOKLAUDE_HOME`;
reads `config.toml` for voice/speed) · `playback` (f32 WAV + `afplay`) · `clean`
(markdown → speakable prose via `pulldown-cmark`) · `transcript` (Stop-hook stdin
JSON → last assistant turn from the session JSONL) · `toggle` (`enabled` flag,
`on`/`off`). `say --voice/--speed` override config (precedence: flag > file >
default). Unit-tested per module; reviewed (one image-alt cleaning bug found + fixed).
`clean`/`transcript`/`is_enabled` are built but unwired (dead-code-allowed) until
Phase 4's hook composes them.

## Phase 4 — Daemon + hook
**Goal:** the hot path. A warm daemon holds the model; the Stop hook is a thin
client that ships the cleaned reply over a unix socket and returns immediately —
playback happens in the daemon, serially, never blocking Claude Code.
- `daemon`: warm engine, unix socket, serial playback **queue**, 30-min idle exit.
- `client`: connect (spawn daemon if absent), send text, never block Claude Code.
- Wire `koklaude hook`: transcript → clean → daemon.
- Integration test: fixture transcript → non-empty audio.

Build it on `std` — `UnixListener`/`UnixStream` + one worker thread + an `mpsc`
queue. No async runtime (KISS; one model, serial playback — tokio earns nothing).

### Slices (working notes — iterate, then delete on phase completion)
Each slice = one reviewable PR-sized step: build → clippy/tests green → review.
IPC framing and the hook pipeline are pure/testable and land first; the daemon
and client (engine-bound, model-gated) follow.

- **4a — socket path + wire protocol.** Add `Config::socket_path()` →
  `<home>/daemon.sock`. New `ipc` module with the frame contract: **one
  connection = one request**; the client writes UTF-8 text then half-closes,
  the daemon reads to EOF. `send(path, text)` + `recv(stream) -> String` helpers.
  Unit-test the round-trip over a real socket in a temp dir (no engine, fast).
- **4b — `daemon` core (listen + queue + serial worker).** `koklaude daemon`:
  bind the socket (fail loud if already bound — one daemon only), load the
  `Engine` once, spawn a worker thread draining an `mpsc<String>` that
  synth→plays serially (reuses `playback::play`). Accept loop reads each
  connection via `ipc::recv` and pushes onto the queue; a slow playback never
  blocks accept. No idle-exit yet. Model-gated smoke test (like `say`).
- **4c — idle shutdown.** Worker uses `recv_timeout(idle)`; on timeout the daemon
  exits to free RAM (`idle_timeout_minutes` in `config.toml`, default 30; Phase 5
  `init` writes it). Clean up the socket file on exit, and recover a stale socket
  on startup (probe-connect: refused = unlink+rebind, live = bail) so a killed
  daemon self-heals on next launch. Test the timeout + bind paths (no model).
- **4d — `client` (connect or spawn).** `connect-or-spawn`: try `ipc::send`; on
  `NotFound`/`ConnectionRefused` (no daemon, or stale socket from a crash),
  spawn `koklaude daemon` **detached** (own session/process group so it outlives
  the hook and CC doesn't wait on it), poll-connect with backoff until ready,
  then send. Returns as soon as the text is handed off — never waits on playback.
  Model-gated smoke test: spawn → send → audio plays.
- **4e — wire `koklaude hook`.** Pure pipeline fn `reply_to_speak(stdin, read_fn)
  -> Option<String>`: parse stdin → `transcript_path` → read JSONL →
  `last_assistant_turn` → `clean`; `None` if disabled or nothing to say. The
  `hook` command runs it, and on `Some` calls the 4d client. **Always exits 0**;
  every error (disabled, no model, daemon unreachable, parse failure) logs to
  stderr and returns success — silence, never a stuck assistant. Fixture-driven
  tests on the pure pipeline (gate on `is_enabled`); reuse the 3d sample.
- **4f — review + docs.** Clippy/tests green end-to-end; manual run: trigger a
  real Stop hook and confirm Claude speaks. Record the wire-protocol + std-only
  decision in `decisions.md`; mark Phase 4 done and prune these notes.

Open within the phase:
- Detaching the daemon on macOS: `Command` + `setsid`/`pre_exec` to escape the
  hook's process group. Confirm the spawned daemon survives the hook exiting
  (the one real portability risk in 4d).
- Stale-socket recovery: file present but no listener (daemon crashed). 4d treats
  `ConnectionRefused` as "respawn"; the daemon must `unlink` then re-bind (4c).
- Long replies: still synth-then-play per request (chunking/streaming stays a
  post-1.0 open question below — don't pull it into Phase 4).

## Phase 5 — Setup / one-command install
- `koklaude init`: download model + voices to `~/.config/koklaude/`, write default
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
