# Prerequisites

What koklaude needs on the machine. **`koklaude init` automates the model/voices
download, config, and Stop-hook registration** (§2 below) — run it once and skip
the manual `curl`. `espeak-ng` (§1) you still install yourself; `init` checks for
it and prints this hint if it's missing. The manual steps below remain the
fallback and document what the Phase 1 spike (`docs/spike.md`) relies on.

`koklaude uninstall` reverses `init` (removes the Stop hook, disables speech);
`--purge` also deletes the downloaded model/voices.

## 1. `espeak-ng` (grapheme→phoneme)

Required at runtime for phonemization. Install:

```sh
# macOS
brew install espeak-ng
# Debian/Ubuntu
sudo apt-get install espeak-ng
```

Verify:

```sh
espeak-ng --version
# eSpeak NG text-to-speech: 1.52.0   (1.52.0 here)
```

## 2. The Kokoro model + voices

`koklaude init` downloads everything automatically; the manual `curl` below is
only for the spike or a hand setup. All assets live under `~/.config/koklaude/`
(koklaude's runtime home — see `docs/architecture.md`) and are **not** committed
to the repo.

**Source:** the official community ONNX repo
[`onnx-community/Kokoro-82M-v1.0-ONNX`](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX)
on Hugging Face — model `onnx/model.onnx`, voices as one `voices/<name>.bin` each.

| Path | What | Size |
|---|---|---|
| `kokoro-v1.0.onnx` | Kokoro-82M weights, fp32 (`onnx/model.onnx`) | ~310 MB |
| `voices/<name>.bin` | one style file per voice — 55 voices total | ~0.5 MB each (~28 MB all) |

```sh
mkdir -p ~/.config/koklaude/voices
base=https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main

curl -fL -o ~/.config/koklaude/kokoro-v1.0.onnx "$base/onnx/model.onnx"
# one voice (init fetches all 55):
curl -fL -o ~/.config/koklaude/voices/af_heart.bin "$base/voices/af_heart.bin"
```

**See list of voices** (after install):

```bash
ls ~/.config/koklaude/voices/ | sed 's/\.bin$//'
```

## 3. Build toolchain

- Rust (stable) + [`just`](https://github.com/casey/just) (`brew install just`).
- `unzip` on PATH (ships with macOS / most Linux) — the spike reads a voice
  straight out of the npz with it.
- **No** system ONNX Runtime needed: `ort` downloads its own binary on first
  build (the `download-binaries` feature).

---

Once these are in place, run the engine spike — see [`docs/spike.md`](spike.md).
