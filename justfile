# koklaude — common commands. Run `just` to list.

default:
    @just --list

# Type-check the whole workspace.
check:
    cargo check --workspace

# Lint with clippy, warnings as errors.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all tests.
test:
    cargo test --workspace

# Format the code.
fmt:
    cargo fmt --all

# Phase 1 engine spike: synth "Hello world" → /tmp/koklaude-spike.wav, then play.
# Needs ~/.claude/koklaude/{kokoro-v1.0.onnx,af_heart.npy}.
spike:
    cargo run -p hanasu --example spike
    afplay /tmp/koklaude-spike.wav
