//! The warm daemon: one Engine held in memory, replies played serially.
//!
//! Binds a unix socket; each connection is one request (`ipc::recv`). Requests
//! go on an `mpsc` queue drained by a single worker thread that synth→plays one
//! at a time — a slow playback never blocks the accept loop, and replies never
//! drop or overlap (decisions D7).
//!
//! Socket lifecycle: std never unlinks the socket on exit, so a `kill`/crash
//! leaves a stale file. We handle both ends — `bind` recovers a stale socket on
//! startup (probe-connect: refused = stale → unlink + rebind; live = bail), and
//! the worker unlinks on graceful idle-exit. Startup recovery is the real safety
//! net (signals/crashes never run cleanup); the unlink is just tidiness.

use std::io::ErrorKind;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hanasu::{Audio, Engine};
use tracing::{error, info};

use crate::config::Config;
use crate::{ipc, playback};

/// Bind the socket, warm the engine, and serve until idle (or killed).
pub fn run(cfg: &Config) -> Result<()> {
    let socket = cfg.socket_path();
    let listener = bind(&socket)?;

    let engine = Engine::load(&cfg.model_path(), &cfg.voices_dir(), &cfg.voice, cfg.speed)
        .context("load engine (is the model present under the koklaude home?)")?;
    info!(voice = %cfg.voice, "daemon started");

    let (tx, rx) = mpsc::channel::<String>();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let idle = cfg.idle_timeout;
    thread::spawn(move || play_loop(engine, rx, socket, idle, done_tx));

    accept_loop(listener, &tx, &done_rx);
    Ok(())
}

/// Bind `socket`, recovering a stale file left by a crashed/killed daemon.
fn bind(socket: &Path) -> Result<UnixListener> {
    match UnixListener::bind(socket) {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            // The file exists. Live daemon, or stale from a kill/crash?
            if UnixStream::connect(socket).is_ok() {
                bail!("daemon already running on {socket:?}");
            }
            // Nothing listening → stale. Remove and rebind.
            std::fs::remove_file(socket)
                .with_context(|| format!("remove stale socket {socket:?}"))?;
            UnixListener::bind(socket).with_context(|| format!("rebind {socket:?}"))
        }
        Err(e) => Err(e).with_context(|| format!("bind {socket:?}")),
    }
}

/// Accept connections forever, pushing each non-empty request onto the queue.
fn accept_loop(listener: UnixListener, tx: &Sender<String>, done: &Receiver<()>) {
    if let Err(e) = listener.set_nonblocking(true) {
        error!(error = %format!("{e:#}"), "set listener nonblocking failed");
        return;
    }

    loop {
        match done.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {}
        }

        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => {
                eprintln!("daemon: accept failed: {e:#}");
                continue;
            }
        };
        match ipc::recv(&mut stream) {
            Ok(text) if !text.trim().is_empty() => {
                if tx.send(text).is_err() {
                    break; // worker gone — nothing left to serve
                }
            }
            Ok(_) => {} // empty request — nothing to say
            Err(e) => error!(error = %format!("{e:#}"), "bad request"),
        }
    }
}

/// Worker thread: drain the queue, then free the model once idle. Exits the
/// whole process (the accept loop is blocked on `incoming()` and can't be
/// unblocked otherwise); the socket is unlinked first so the next spawn binds
/// fresh.
fn play_loop(
    engine: Engine,
    rx: Receiver<String>,
    socket: PathBuf,
    idle: Duration,
    done: Sender<()>,
) {
    drain_until_idle(&rx, idle, |text| speak_one(&engine, &text));
    info!("daemon idle, exiting");
    let _ = std::fs::remove_file(&socket);
    let _ = done.send(());
}

/// Synth + play one reply in chunks, so replies longer than Kokoro's 510-phoneme
/// window are spoken in full instead of being silently truncated. The daemon's
/// stderr is `/dev/null`, so these `tracing` events are the only trace of what
/// it actually did.
fn speak_one(engine: &Engine, text: &str) {
    let chars = text.chars().count();
    let t0 = Instant::now();

    let chunks = match engine.text_chunks(text) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %format!("{e:#}"), chars, "text chunking failed");
            return;
        }
    };
    let n = chunks.len();
    info!(
        chars,
        chunks = n,
        chunking_ms = t0.elapsed().as_millis() as u64,
        "speech queued"
    );

    if let Err(e) = speak_chunks_pipelined(engine, &chunks, chars) {
        error!(error = %format!("{e:#}"), chars, chunks = n, "speech failed");
        return;
    }

    info!(
        chars,
        chunks = n,
        total_ms = t0.elapsed().as_millis() as u64,
        "spoke reply"
    );
}

/// Start playback as soon as the first chunk is synthesized, then synthesize one
/// chunk ahead while the current chunk is playing.
fn speak_chunks_pipelined(engine: &Engine, chunks: &[String], chars: usize) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }

    thread::scope(|scope| -> Result<()> {
        let (mut audio, mut synth_ms) = synth_timed(engine, &chunks[0])?;

        for i in 0..chunks.len() {
            let next = chunks.get(i + 1).map(|chunk| {
                let chunk = chunk.as_str();
                scope.spawn(move || synth_timed(engine, chunk))
            });

            let play_t0 = Instant::now();
            playback::play(&audio).with_context(|| format!("play chunk {i}"))?;
            let play_ms = play_t0.elapsed().as_millis() as u64;
            info!(
                chars,
                chunk = i,
                chunks = chunks.len(),
                chunk_chars = chunks[i].chars().count(),
                synth_ms,
                play_ms,
                "spoke chunk"
            );

            if let Some(handle) = next {
                (audio, synth_ms) = handle
                    .join()
                    .map_err(|_| anyhow!("synth worker panicked"))??;
            }
        }

        Ok(())
    })
}

fn synth_timed(engine: &Engine, text: &str) -> Result<(Audio, u64)> {
    let t0 = Instant::now();
    let audio = engine.synth(text).context("synthesize chunk")?;
    Ok((audio, t0.elapsed().as_millis() as u64))
}

/// Drain `rx`, calling `play` for each request, until no request arrives within
/// `idle` (or the queue disconnects). Pure control flow — no engine, no exit —
/// so the idle decision is unit-testable.
fn drain_until_idle<F: FnMut(String)>(rx: &Receiver<String>, idle: Duration, mut play: F) {
    loop {
        match rx.recv_timeout(idle) {
            Ok(text) => play(text),
            Err(_) => return, // Timeout or Disconnected → stop serving
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh socket path under temp (removed if a prior run left one).
    fn scratch_sock(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-daemon-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("daemon.sock");
        let _ = std::fs::remove_file(&sock);
        sock
    }

    /// Plumbing only — no engine, no audio. Proves the accept loop frames each
    /// connection and enqueues it in order; play_loop (engine+afplay) is covered
    /// by hanasu's synth smoke and 4d's spawn→send→audio test.
    #[test]
    fn accept_loop_enqueues_in_order() {
        let sock = scratch_sock("order");
        let listener = UnixListener::bind(&sock).unwrap();
        let (tx, rx) = mpsc::channel();
        let (_done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || accept_loop(listener, &tx, &done_rx));

        ipc::send(&sock, "first reply").unwrap();
        ipc::send(&sock, "second reply").unwrap();

        let got = |rx: &Receiver<String>| rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(got(&rx), "first reply");
        assert_eq!(got(&rx), "second reply");
    }

    #[test]
    fn accept_loop_skips_empty_requests() {
        let sock = scratch_sock("empty");
        let listener = UnixListener::bind(&sock).unwrap();
        let (tx, rx) = mpsc::channel();
        let (_done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || accept_loop(listener, &tx, &done_rx));

        ipc::send(&sock, "  \n ").unwrap(); // whitespace only → skipped
        ipc::send(&sock, "real").unwrap();

        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "real");
    }

    #[test]
    fn drain_returns_when_idle() {
        let (_tx, rx) = mpsc::channel::<String>(); // _tx alive → tests the Timeout path, not Disconnected
        let mut played: Vec<String> = Vec::new();
        drain_until_idle(&rx, Duration::from_millis(100), |t| played.push(t));
        assert!(played.is_empty());
    }

    #[test]
    fn drain_plays_queued_then_idles() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("a".into()).unwrap();
        tx.send("b".into()).unwrap();
        let mut played: Vec<String> = Vec::new();
        drain_until_idle(&rx, Duration::from_millis(100), |t| played.push(t));
        assert_eq!(played, ["a", "b"]);
    }

    #[test]
    fn bind_recovers_stale_socket() {
        let sock = scratch_sock("stale");
        // A bound-then-dropped listener leaves the file behind (std never unlinks).
        drop(UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());

        let listener = bind(&sock).expect("recover stale socket");
        let (tx, rx) = mpsc::channel();
        let (_done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || accept_loop(listener, &tx, &done_rx));
        ipc::send(&sock, "alive").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "alive");
    }

    #[test]
    fn bind_refuses_when_live() {
        let sock = scratch_sock("live");
        let _live = UnixListener::bind(&sock).unwrap(); // kept listening
        assert!(bind(&sock).is_err());
    }
}
