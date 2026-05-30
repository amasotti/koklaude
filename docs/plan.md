# Plan

koklaude went from an empty scaffold to a working, installable, offline TTS for
Claude Code — built iteratively in small, reviewable slices, clippy-clean and
tested at each step. This is the trail of what shipped; the live engineering
detail lives in the docs linked per phase.

## Shipped

- **Phase 0 — Scaffold.** Cargo workspace: `hanasu` (engine lib) + `koklaude`
  (binary); CLI skeleton; docs; green `cargo check`.
- **Phase 1 — Engine spike.** Proved text → audible WAV through the full chain
  (espeak g2p → tokenize → `ort` → WAV) and pinned the Kokoro ONNX I/O contract.
  Repro: [`spike.md`](spike.md).
- **Phase 2 — `hanasu` engine API.** `Engine::load(…).synth(text) -> Audio`;
  espeak g2p (punctuation-preserving) → tokenize (≤ 510) → `ort` → samples. 17
  tests. Spec: [`phase2-engine-api.md`](phase2-engine-api.md).
- **Phase 3 — `koklaude` front end.** Pure, tested modules — `config`, `playback`,
  `clean` (markdown → prose), `transcript` (Stop-hook → last turn), `toggle`.
  `koklaude say` works end to end, no daemon.
- **Phase 4 — Daemon + hook.** Warm daemon over a unix socket; the Stop hook is a
  thin client returning in ~11 ms warm / ~115 ms cold; serial playback never
  blocks Claude Code; stale sockets self-heal. Deep dive:
  [`daemon-and-sockets.md`](daemon-and-sockets.md).
- **Phase 5 — One-command install.** `koklaude init` (detect espeak → download
  model + voices → write config → register the Stop hook → enable) and a
  symmetric `koklaude uninstall`; atomic writes, never clobbers foreign hooks.
- **Phase 6 — Polish & ship.** Default voice `af_heart`; README made truthful;
  macOS release pipeline (git-cliff → release PR → merge → tag → GitHub Release
  with the `aarch64-apple-darwin` binary — [`release.md`](release.md)); demo
  playbook ([`demo.md`](demo.md)).

## Next versions

- **Extract `hanasu`** to its own repo (`git subtree split`) and publish to
  crates.io — the maintained Kokoro engine on `ort` 2.0 the ecosystem lacks
  (successor to the dead `kokoroxide`; MIT). Includes picking the crates.io name
  (free today) and wiring `cargo install koklaude` from the registry.
- **More assistants** — Codex / pi adapters: a new thin front end per assistant,
  same engine (architecture › Extensibility).
- **Linux / Windows playback** beyond macOS `afplay`.
- **Streaming / chunked synthesis** for long replies (today: synth-then-play per
  request).
