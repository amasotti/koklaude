//! koklaude — local, offline text-to-speech for Claude Code.
//!

mod clean;
mod config;
mod playback;
mod transcript;

use anyhow::Context;
use clap::{Parser, Subcommand};
use config::Config;
use hanasu::Engine;

#[derive(Parser)]
#[command(
    name = "koklaude",
    version,
    about = "Local offline TTS for Claude Code — Claude speaks its replies."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// One-time setup: download the model and register the Stop hook.
    Init,
    /// Run the background daemon (holds the model warm).
    Daemon,
    /// Stop-hook entrypoint. Reads hook JSON from stdin.
    Hook,
    /// Enable speech.
    On,
    /// Disable speech.
    Off,
    /// Speak arbitrary text (manual test / standalone playback).
    Say {
        text: String,
        /// Override the configured voice (e.g. `am_adam`).
        #[arg(long)]
        voice: Option<String>,
        /// Override the configured speed (1.0 = normal).
        #[arg(long)]
        speed: Option<f32>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => todo!("init: download model + register Stop hook"),
        Command::Daemon => todo!("daemon: warm model + unix socket + play queue"),
        Command::Hook => todo!("hook: parse transcript -> clean -> daemon"),
        Command::On => todo!("on: create enabled flag"),
        Command::Off => todo!("off: remove enabled flag"),
        Command::Say { text, voice, speed } => say(&text, voice, speed),
    }
}

/// Synthesize `text` with the configured engine and play it. No daemon —
/// loads the model fresh each call (a manual test path, not the hot path).
/// `voice`/`speed` override config when given (precedence: flag > file > default).
fn say(text: &str, voice: Option<String>, speed: Option<f32>) -> anyhow::Result<()> {
    let mut cfg = Config::load()?;
    if let Some(v) = voice {
        cfg.voice = v;
    }
    if let Some(s) = speed {
        cfg.speed = s;
    }
    let engine = Engine::load(&cfg.model_path(), &cfg.voices_path(), &cfg.voice, cfg.speed)
        .context("load engine (is the model present under ~/.claude/koklaude/?)")?;
    let audio = engine.synth(text).context("synthesize text")?;
    playback::play(&audio)
}
