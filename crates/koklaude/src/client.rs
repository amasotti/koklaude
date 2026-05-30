//! Front-end side of the socket: ship a reply to the daemon, spawning it if
//! absent. The hook calls this; it must return as soon as the text is handed
//! off — playback happens in the daemon, never blocking Claude Code.
//!
//! "Daemon absent" = connect fails with `NotFound` (no socket file) or
//! `ConnectionRefused` (stale socket from a crash; the daemon's `bind` unlinks
//! and rebinds it on respawn). Any other connect error is a real failure.

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::ipc;

/// Poll the freshly-spawned daemon this many times, this far apart, before
/// giving up. The daemon binds the socket *before* loading the model, so it
/// becomes connectable within a few ms; the request then buffers until the
/// engine is warm. 1s total is generous — and bounds how long a broken install
/// (daemon never binds) can stall the hook, honouring "never block Claude Code".
const RETRIES: u32 = 20;
const INTERVAL: Duration = Duration::from_millis(50);

/// Send `text` to the daemon at `socket`, spawning it if not yet running.
pub fn send(socket: &Path, text: &str) -> Result<()> {
    match UnixStream::connect(socket) {
        Ok(mut stream) => ipc::write_request(&mut stream, text),
        Err(e) if is_absent(&e) => {
            spawn_daemon()?;
            let mut stream = connect_with_retry(socket, RETRIES, INTERVAL)?;
            ipc::write_request(&mut stream, text)
        }
        Err(e) => Err(e).with_context(|| format!("connect {socket:?}")),
    }
}

/// Does this connect error mean "no daemon" (vs a genuine failure)?
fn is_absent(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused)
}

/// Spawn `koklaude daemon` detached: stdio to /dev/null so Claude Code's pipe to
/// the hook closes when the hook exits, regardless of the long-lived daemon.
fn spawn_daemon() -> Result<()> {
    let exe = std::env::current_exe().context("locate the koklaude binary")?;
    Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn koklaude daemon")?;
    Ok(())
}

/// Retry-connect until the daemon is listening (or we run out of tries).
fn connect_with_retry(socket: &Path, retries: u32, interval: Duration) -> Result<UnixStream> {
    for _ in 0..retries {
        if let Ok(stream) = UnixStream::connect(socket) {
            return Ok(stream);
        }
        thread::sleep(interval);
    }
    bail!("daemon did not become ready at {socket:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    fn scratch_sock(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-client-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("daemon.sock");
        let _ = std::fs::remove_file(&sock);
        sock
    }

    #[test]
    fn is_absent_only_for_missing_or_refused() {
        use std::io::Error;
        assert!(is_absent(&Error::from(ErrorKind::NotFound)));
        assert!(is_absent(&Error::from(ErrorKind::ConnectionRefused)));
        assert!(!is_absent(&Error::from(ErrorKind::PermissionDenied)));
    }

    #[test]
    fn sends_to_a_running_daemon_without_spawning() {
        let sock = scratch_sock("running");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            ipc::recv(&mut stream).unwrap()
        });

        send(&sock, "hello").unwrap();
        assert_eq!(server.join().unwrap(), "hello");
    }

    #[test]
    fn retry_connect_succeeds_once_listener_appears() {
        let sock = scratch_sock("appears");
        let sock_for_server = sock.clone();
        // Listener shows up mid-poll — proves we retry, not just try once.
        let server = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            let listener = UnixListener::bind(&sock_for_server).unwrap();
            listener.accept().unwrap(); // hold until the client connects
        });

        let stream = connect_with_retry(&sock, 30, Duration::from_millis(50));
        assert!(stream.is_ok());
        server.join().unwrap();
    }

    #[test]
    fn retry_connect_gives_up_when_no_daemon() {
        let sock = scratch_sock("nodaemon"); // nothing ever binds
        let r = connect_with_retry(&sock, 2, Duration::from_millis(10));
        assert!(r.is_err());
    }
}
