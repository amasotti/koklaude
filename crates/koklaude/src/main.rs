//! koklaude — local, offline text-to-speech for Claude Code.
//!

use clap::{Parser, Subcommand};

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
    /// Speak arbitrary text (manual test).
    Say { text: String },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => todo!("init: download model + register Stop hook"),
        Command::Daemon => todo!("daemon: warm model + unix socket + play queue"),
        Command::Hook => todo!("hook: parse transcript -> clean -> daemon"),
        Command::On => todo!("on: create enabled flag"),
        Command::Off => todo!("off: remove enabled flag"),
        Command::Say { text } => todo!("say: speak {text:?}"),
    }
}
