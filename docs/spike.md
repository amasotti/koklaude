# Phase 1 engine spike — reproduction

The spike (`crates/hanasu/examples/spike.rs`) proves the full chain end to end on
one machine, and pins Kokoro-82M's ONNX I/O contract. This is how to reproduce
exactly what we ran. It is a **throwaway** — the real engine lands in Phase 2.

```
text → [espeak-ng] → IPA phonemes → [Kokoro vocab] → token ids → [ONNX] → audio (WAV)
```

## Prerequisites

| Need | Why | Get it (macOS) |
|---|---|---|
| `espeak-ng` on PATH | grapheme→phoneme (g2p) | `brew install espeak-ng` |
| `unzip` on PATH | reads a voice out of the voices npz | ships with macOS |
| Rust toolchain + `just` | build/run | `brew install just` |
| `kokoro-v1.0.onnx` | the model (fp32, ~310 MB) | download ↓ |
| `voices-v1.0.bin` | 54 voice style vectors (~28 MB) | download ↓ |

`ort` downloads its own ONNX Runtime binary on first build (the
`download-binaries` feature) — **no** `brew install onnxruntime` needed.

Verify espeak is present:

```sh
espeak-ng --version
# eSpeak NG text-to-speech: 1.52.0  (was 1.52.0 when we ran this)
```

## Download the model + voices

Both files live outside the repo, under `~/.claude/koklaude/` (the project's real
runtime layout — see `docs/architecture.md`). They are **not** committed.

```sh
mkdir -p ~/.claude/koklaude
cd ~/.claude/koklaude

# Model weights, fp32 (~310 MB) and the packed voices (~28 MB).
# Source: thewh1teagle/kokoro-onnx release `model-files-v1.0`.
curl -fL -o kokoro-v1.0.onnx \
  https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx
curl -fL -o voices-v1.0.bin \
  https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin
```

`voices-v1.0.bin` is a zip (npz) of `<voice>.npy` files, each a numpy `<f4`
array of shape `(510, 1, 256)`. The spike reads one voice straight from it with
`unzip -p` — no manual extraction.

## Run

```sh
just spike                                   # voice af_heart, "Hello world"
just spike bm_george "Good evening"          # pick any voice + text
# or directly:
cargo run -p hanasu --example spike -- am_michael "Tests are green"
```

`just spike` runs the example then `afplay`s `/tmp/koklaude-spike.wav`.

## Expected output (the de-risk)

The spike prints the model's real contract — this is the unknown it pins:

```
=== ONNX inputs ===
  tokens       Int64   [1, -1]      ← NOT `input_ids` (the common assumption)
  style        Float32 [1, 256]
  speed        Float32 [1]
=== ONNX outputs ===
  audio        Float32 [-1]         ← NOT `waveform`; mono PCM @ 24 kHz
```

And the chain, with a self-check that live g2p reproduces our verified ids:

```
[g2p]   espeak-ng en-us → "həlˈoʊ wˈɜːld"
[token] 13 ids: [50, 83, 54, 156, 57, 135, 16, 65, 156, 87, 158, 54, 46]
[check] live g2p vs known-good [50,83,…]: ✓ match
...
output 'audio' shape=[39000], 39000 samples
```

"Hello world" → 39000 samples = 1.62 s. Done when you hear it clearly.

## Available voices (54)

Read straight from the npz; prefix = language, then `f`/`m` = female/male:

```sh
unzip -l ~/.claude/koklaude/voices-v1.0.bin | grep -o '[a-z][a-z]_[a-z]*'
```

| Prefix | Language | Examples |
|---|---|---|
| `a` | American English | `af_heart`, `af_bella`, `am_michael`, `am_adam` |
| `b` | British English | `bf_emma`, `bm_george`, `bm_lewis` |
| `e` | Spanish | `ef_dora`, `em_alex` |
| `f` | French | `ff_siwis` |
| `h` | Hindi | `hf_alpha`, `hm_omega` |
| `i` | Italian | `if_sara`, `im_nicola` |
| `j` | Japanese | `jf_alpha`, `jm_kumo` |
| `p` | Portuguese (BR) | `pf_dora`, `pm_alex` |
| `z` | Chinese | `zf_xiaobei`, `zm_yunjian` |

## Known limitations (by design — Phase 2 fixes these)

The spike's tokenizer is the **naive** char→id map, enough to *see* the flow,
not the final one:

- **g2p is fixed to `en-us`.** Non-English voices still get English phonemes — a
  timbre swap, not correct foreign pronunciation. Speaking, say, Japanese
  properly means phonemizing with the matching espeak language.
- **No punctuation/clause normalization.** espeak emits a newline on clause
  breaks (e.g. after a comma); the naive map skips unknown chars and prints a
  warning. Real Misaki normalization (stress handling, multi-char phonemes,
  clause pauses) is built **with unit tests** in Phase 2.

The verified ONNX contract above is final and carries into the real engine.
