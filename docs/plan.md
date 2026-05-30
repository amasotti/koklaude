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

## Phase 4 — Daemon + hook ✅
The hot path: a warm daemon holds the model and the Stop hook is a thin client
that ships the cleaned reply over a unix socket and returns in ~11 ms (warm) /
~115 ms (cold spawn) — playback happens in the daemon, serially, never blocking
Claude Code. Built on `std` (`UnixListener`/`UnixStream` + one worker thread +
an `mpsc` queue; no async runtime). New modules in the binary: `ipc` (EOF-framed
wire protocol, one connection = one request) · `daemon` (bind with stale-socket
recovery → warm `Engine` → accept loop → serial playback worker; idle exit via
`recv_timeout`, `idle_timeout_minutes` in `config.toml`, default 30) · `client`
(connect-or-spawn: spawn the daemon detached via stdio→/dev/null, poll-connect
~1 s, send) · `hook` (pure `reply_to_speak` pipeline; **always exits 0** — every
error logs to stderr, worst case is silence). Killed/crashed daemons self-heal on
next launch (probe-connect → unlink stale socket → rebind). Reviewed (cut the
client retry budget to ~1 s; gated the test-only `ipc::send`). Decisions D10/D11;
deep dive [`daemon-and-sockets.md`](daemon-and-sockets.md).

Carried forward (post-1.0): long replies still synth-then-play per request;
chunking/streaming stays an open question below.

## Phase 5 — Setup / one-command install ✅
`koklaude init` takes a fresh machine to "Claude speaks" in one command, and
`koklaude uninstall` cleanly reverses it — never touching the user's other hooks.
All in one new `setup` module. `init`: detect `espeak-ng` (print the install hint
if missing, don't fail) → download model + voices → write default `config.toml` →
merge the Stop hook into `~/.claude/settings.json` → `toggle::enable`.
`uninstall`: strip the koklaude hook → `toggle::disable` (`--purge` also removes
the koklaude home; model/voices kept by default — re-download is expensive).
- **Settings surgery is pure + symmetric** (`merge_stop_hook`/`remove_stop_hook`
  over `serde_json::Value`): idempotent merge, cascade-cleaning remove, errors
  rather than clobbering a mis-shaped `hooks`/`Stop`. Verified schema: `hooks.Stop`
  is an array of `{ "hooks": [...] }` groups, **no `matcher`** (code.claude.com/docs).
- **Downloads** stream via `ureq` `into_reader()` (unlimited — the 10 MB default cap
  would truncate the 310 MB model) to a `.part` temp then rename; skip if present
  non-empty; a truncated fetch fails the read and never renames. Settings writes are
  atomic the same way — temp + rename, so a crash can't corrupt `settings.json`.
- The hook command is registered as the binary's **absolute path** + `hook` (works
  regardless of `$PATH`); `~/.claude` honors `$CLAUDE_CONFIG_DIR`.
- 11 unit tests (merge/remove round-trips, config write, settings rewrite, `.part`
  plumbing) + one `#[ignore]` network smoke test. Manual init→uninstall through the
  binary confirmed settings.json **byte-restored**, foreign hooks untouched. Docs:
  `prerequisites.md` now points at `init`.

Carried into Phase 6: pick a good default voice; the README "Install & use".

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
