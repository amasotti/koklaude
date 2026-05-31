# koklaude

<p align="center">
    <img src="docs/koklaude.png" width="350"/>
</p>

**Local, offline text-to-speech for Claude Code.** 
Claude finishes a reply — and *speaks* it aloud, on your machine, with no cloud, no subscriptions and no API keys.

It uses the open-weight [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) TTS model, based on the [StyleTTS 2](https://arxiv.org/abs/2306.07691)
family of models.

> Status: **working on macOS.**
> The full loop is wired — `koklaude init` sets it up, a warm daemon synthesizes, and the
> Stop hook makes Claude speak each reply. `koklaude say "..."` works standalone too.
> Remaining polish: prebuilt release binaries and a demo. macOS-only for now (playback is `afplay`).

---

## Why

Coding and especially brainstorming with an assistant is a read-heavy loop: 
you skim a wall of text, often in a small terminal, find the one sentence or idea that matters, then act, correct, reiterate. 
There are already many good options to "speak" **to** coding agents, including some built-in features and plugin, but there is - at least to my knowledge - very little in the
other direction: let the assistant speak loud the answer.

Koklaude (kokoro + claude) turns the assistant's reply into *audio* so you can keep your eyes on the editor (or look away entirely) 
and still follow what it did. Pair it with any speech-to-text input (Claude Code's built-in voice mode, Spokenly, Whisper) and the loop becomes conversational.

Three hard requirements shaped every decision:

1. **Safe** — runs fully on-device. Your code and the assistant's replies never leave the machine.
2. **Free & local** — the [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) model runs locally via ONNX. No subscription, no key.
3. **Toggleable** — flip speech on/off instantly (`koklaude on` / `koklaude off`), no uninstall, no restart.

## Why *another* TTS-for-Claude project

There's already a couple of good projects in this direction, e.g. [`ybouhjira/claude-code-tts`](https://github.com/ybouhjira/claude-code-tts) (Go) and a few Rust Kokoro wrappers. I looked at each before writing a line:

| Project                     | What it is                              | Why not for this                                                                                                                             |
|-----------------------------|-----------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------|
| `ybouhjira/claude-code-tts` | Go plugin, Stop hook + worker pool      | Uses the **OpenAI cloud TTS API** — pay to use, and sends every reply to a third party. Fails "safe" and "free/local". Also hard to turn-off |
| `kokoroxide` (crate)        | MIT/Apache, clean in-process lib API    | **Dead**: pins `ort = "^1.16"`, and every `ort 1.16.x` is yanked. Uninstallable, ~8 months stale.                                            |
| `kokorox` / `Kokoros`       | Rust Kokoro, installable (`ort 2.0-rc`) | Shaped as a CLI/server, not a clean library.                                                                                                 |

None met *safe, free/local, small, embeddable*. 
So koklaude rebuilds what `kokoroxide` set out to be — a clean Kokoro engine on a maintained `ort` 2.0 — as **`hanasu`**. Like every Kokoro stack that pronounces arbitrary words well, it uses `espeak-ng` for phonemes — but invoked as a **separate CLI process**, so koklaude itself stays **MIT** (see [License](#license)).

## How it works (one binary)

One Stop hook, a warm daemon, and the `hanasu` engine. The full flow — from
Claude's reply to audio — is diagrammed in
[`docs/architecture.md`](docs/architecture.md).

- **`espeak-ng`** (invoked as an external CLI) — grapheme→phoneme for arbitrary words, names, jargon, and many languages. This is what lets koklaude pronounce real-world, non-English, domain-heavy text correctly. You install it yourself ([prerequisites](docs/prerequisites.md)); calling it arm's-length keeps koklaude MIT.
- **`ort` 2.0** — runs the Kokoro ONNX model.
- A small background **daemon** keeps the model loaded so speech starts fast; it auto-spawns on first use and exits after 30 min idle.

Full detail: [`docs/architecture.md`](docs/architecture.md).

## Install & use

**Option A — prebuilt binary** (macOS, Apple Silicon). Each release attaches an
`aarch64-apple-darwin` tarball; grab the latest with the GitHub CLI:

```bash
gh release download --repo amasotti/koklaude \
  --pattern '*-aarch64-apple-darwin.tar.gz'
tar -xzf koklaude-*-aarch64-apple-darwin.tar.gz
sudo mv koklaude /usr/local/bin/        # or anywhere on your PATH
```

The binary isn't notarized. If you download it through a browser instead of
`gh`, macOS Gatekeeper will quarantine it — clear that with
`xattr -d com.apple.quarantine /usr/local/bin/koklaude`.

**Option B — from source** (any cargo target; `cargo install koklaude` from
crates.io is coming — see [`docs/plan.md`](docs/plan.md)):

```bash
git clone https://github.com/amasotti/koklaude && cd koklaude
cargo install --path crates/koklaude
```

Then, either way:

```bash
koklaude init                 # check for espeak-ng, download model + voices,
                              # write config, register the Stop hook
# ... that's it. Claude now speaks.

koklaude off                  # silence
koklaude on                   # speech back
koklaude say "hello there"    # manual test (standalone, no daemon)
koklaude uninstall            # cleanly remove the hook (your other hooks untouched)
```

`init` automates everything except `espeak-ng` itself — you install that once (see
[Prerequisites](#prerequisites)); `init` prints the one-line install hint if it's missing.

### Standalone playback mode

`koklaude` isn't only a Claude Code hook — `koklaude say "..."` is a self-contained
TTS player: it synthesizes the text and plays it straight through your speakers,
no daemon, no hook, no Claude involved. Useful as a quick local
text-to-speech command in its own right (and how we validate the engine).

```bash
koklaude say "Local, offline text to speech in one command."
```

Voice and speed are configurable — globally via `config.toml` and per-call via
`say --voice <name> --speed <n>` (a flag overrides the file). See
[Configuration](#configuration).

### Prerequisites

`koklaude init` downloads the **Kokoro model + voices** into `~/.config/koklaude/`
for you. The one thing it can't install is **`espeak-ng`** (the grapheme→phoneme
backend, kept arm's-length to stay MIT) — install that yourself first, e.g.
`brew install espeak-ng`. Details: [`docs/prerequisites.md`](docs/prerequisites.md).

## Configuration

Speech settings live in a TOML file; paths are overridable with environment
variables.

### `config.toml`

At `~/.config/koklaude/config.toml` (written by `koklaude init`). Every key is
optional — omit one and the built-in default applies. Edit any time.

```toml
voice = "af_heart"          # any of the 54 Kokoro voices (e.g. am_adam, bf_emma)
speed = 1.0                 # pace multiplier; 1.0 = normal
idle_timeout_minutes = 30   # daemon frees the model after this long idle
```

`say --voice <name> --speed <n>` overrides the file per call. Precedence:
`--flag` > `config.toml` > built-in default.

### Environment variables

| Variable | Default | What it controls |
|---|---|---|
| `KOKLAUDE_HOME` | `~/.config/koklaude` | koklaude's home — model, voices, `config.toml`, the `enabled` flag, and the daemon socket. Set it to relocate all state. |
| `KOKLAUDE_LOG_DIR` | `~/.koklaude/logs` | Where the daily JSON logs are written. See [`docs/logging.md`](docs/logging.md). |
| `CLAUDE_CONFIG_DIR` | `~/.claude` | Claude Code's config dir — where `init`/`uninstall` add/remove the Stop hook in `settings.json`. (Claude Code's own variable; koklaude honours it.) |

Logs sit under `~/.koklaude/`, **not** the config home — they're runtime output,
not configuration.

## Design at a glance

- **Speaks** the full reply with code blocks stripped (code read aloud is noise).
- **Never drops text**: overlapping replies queue rather than interrupt — losing half a sentence is worse than slightly stale audio.
- **Never blocks Claude Code**: any TTS error is logged and swallowed; the hook always exits cleanly.
- State lives under `~/.config/koklaude/` (model, voices, config, socket); logs under `~/.koklaude/logs/`.

## Beyond Claude Code

The speech engine (daemon) is assistant-agnostic. 
Only the thin front end — the hook plus the transcript parser — is specific to Claude Code.
Supporting **Codex**, **pi**, or another assistant later means adding a small adapter, not a new engine. 

## License

**MIT.** Use koklaude and the `hanasu` engine freely.

koklaude doesn't bundle or link `espeak-ng` — it calls the separately installed `espeak-ng` as an **external CLI** (the way MIT tools shell out to `git` or `ffmpeg`), so espeak's GPL doesn't propagate. **You install `espeak-ng` yourself** — see [`docs/prerequisites.md`](docs/prerequisites.md). Rationale in [`docs/decisions.md`](docs/decisions.md) (D3/D4). *Not legal advice.*

## Acknowledgements

- [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) by hexgrad 
- [`espeak-ng`](https://github.com/espeak-ng/espeak-ng) 
- [`ort`](https://github.com/pykeio/ort) by pykeio 

