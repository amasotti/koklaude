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

## Phase 5 — Setup / one-command install
**Goal:** `koklaude init` takes a fresh machine to "Claude speaks" in one command:
detect `espeak-ng` → fetch model + voices → write default config → register the
Stop hook → enable. Idempotent: safe to re-run, never clobbers user edits.
`koklaude uninstall` is the clean inverse — pulls the hook back out of
`settings.json` and disables, never breaking the user's other hooks.
- `koklaude init`: download model + voices to `~/.config/koklaude/`, write default
  config, **merge** the Stop hook into `~/.claude/settings.json` (preserving
  existing hooks), enable.
- `koklaude uninstall`: **remove** the koklaude Stop hook from `settings.json`
  (leaving every other hook intact), disable speech. Leaves the downloaded
  model/voices in place by default (re-download is expensive); `--purge` also
  removes the koklaude home.
- Detect `espeak-ng`; if missing, print the install hint (`brew install espeak-ng`).

Deps already in the tree: `ureq` (download), `serde_json` (settings merge),
`dirs` (locate `~/.claude`). No new crates expected.

### Slices (working notes — iterate, then delete on phase completion)
Pure/testable logic (settings merge, config write) lands first; the network
download (heavy, gated like the model smoke tests) and the `init` wiring follow.

- **5a — settings.json merge/unmerge (pure).** New `setup` module, two symmetric fns
  over `serde_json::Value`: `merge_stop_hook(settings, command) -> Value` appends a
  Stop hook group preserving every existing hook, **idempotent** (no duplicate if the
  command is already present anywhere in `Stop`); `remove_stop_hook(settings, command)
  -> Value` strips that command from every Stop group, drops emptied groups / `Stop`
  / `hooks`, leaves all else intact. Verified schema (docs/hooks): `hooks.Stop` is an
  array of groups, each `{ "hooks": [ { "type": "command", "command": "…" } ] }`,
  **no `matcher`** on Stop. Unit-test fixtures both ways: empty `{}`, unrelated hooks
  present, koklaude hook already present (merge no-op / remove cleans), non-object
  `hooks` or non-array `Stop` (error, don't corrupt). No filesystem yet.
- **5b — config.toml write + espeak detection.** `ConfigFile` gains `Serialize`;
  `write_default_config(home)` serializes the defaults **only if absent** (never
  clobber a user-edited file). `espeak_installed() -> bool` via
  `Command::new("espeak-ng").arg("--version")`; on `false`, `init` prints the
  `brew install espeak-ng` hint. Test config-write (present vs absent) in a temp dir.
- **5c — download model + voices (gated).** `download(url, dest)` with `ureq`:
  stream to a `.part` temp then rename (no half-file on interrupt), **skip if dest
  already present and non-empty**, byte progress to stderr. URLs + sizes from
  `docs/prerequisites.md` (`kokoro-onnx` release `model-files-v1.0`). Network-gated
  smoke test (like the model tests); size sanity check, not a full checksum.
- **5d — wire `init` + `uninstall` + review/docs.** `init`: detect espeak → ensure
  home → download model+voices → write config → read/merge/atomically-write
  `~/.claude/settings.json` → `toggle::enable`. `uninstall`: read → `remove_stop_hook`
  → atomic-write → `toggle::disable` (`--purge` also removes the home). Replace the
  `main.rs` `todo!`, add the `Uninstall` subcommand. Clippy/tests green; manual run
  init→uninstall from a clean home, confirming settings.json is byte-restored. Update
  `prerequisites.md` (manual → automated), mark Phase 5 done, prune these notes.

Open within the phase:
- ~~**Stop-hook JSON shape**~~ — ✅ resolved (above): `hooks.Stop` = array of
  `{ "hooks": [...] }` groups, no `matcher`. Source: code.claude.com/docs hooks.
- **Atomic settings write** — read → merge → write temp + rename, so a crash
  mid-write never corrupts the user's `~/.claude/settings.json`.
- **`~/.claude` location** — `dirs::home_dir()` + `.claude`; honor `CLAUDE_CONFIG_DIR`
  if Claude Code sets one (check before hardcoding).
- **Re-run idempotency** — init twice = no duplicate hook, no clobbered config; only
  missing pieces get filled.

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
