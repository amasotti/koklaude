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

`koklaude init` downloads both automatically; the manual `curl` below is only for
the spike or a hand setup. Both live under `~/.config/koklaude/` (koklaude's
runtime home — see `docs/architecture.md`) and are **not** committed to the repo.

| File | What | Size |
|---|---|---|
| `kokoro-v1.0.onnx` | Kokoro-82M weights, fp32 | ~310 MB |
| `voices-v1.0.bin` | 54 voice style vectors (npz) | ~28 MB |

```sh
mkdir -p ~/.config/koklaude
cd ~/.config/koklaude

# Source: thewh1teagle/kokoro-onnx release `model-files-v1.0`.
curl -fL -o kokoro-v1.0.onnx \
  https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx
curl -fL -o voices-v1.0.bin \
  https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin
```

**See list of voices**:

```bash
unzip -l ~/.config/koklaude/voices-v1.0.bin | grep -o '[a-z][a-z]_[a-z]*'
```

## 3. Build toolchain

- Rust (stable) + [`just`](https://github.com/casey/just) (`brew install just`).
- `unzip` on PATH (ships with macOS / most Linux) — the spike reads a voice
  straight out of the npz with it.
- **No** system ONNX Runtime needed: `ort` downloads its own binary on first
  build (the `download-binaries` feature).

---

Once these are in place, run the engine spike — see [`docs/spike.md`](spike.md).
