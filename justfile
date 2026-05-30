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

build:
    cargo build

release:
    cargo build --release

# Phase 1 engine spike: text → phonemes → tokens → audio, then play.
# Needs ~/.config/koklaude/{kokoro-v1.0.onnx,voices-v1.0.bin} + espeak-ng + unzip.
# Defaults to voice `af_heart`, text "Hello world". Examples:
#   just spike
#   just spike bm_george "Good evening"
spike voice="af_heart" text="Hello world":
    cargo run -p hanasu --example spike -- {{voice}} {{text}}
    afplay /tmp/koklaude-spike.wav
