//! Wire protocol between the hook client and the daemon over a unix socket.
//!
//! Frame contract: **one connection = one request**. The client writes the
//! UTF-8 text and half-closes its write half; the daemon reads to EOF. No
//! length prefix, no delimiter — EOF *is* the frame boundary (one request per
//! connection, so there's nothing to disambiguate). Fire-and-forget: no reply.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

use anyhow::{Context, Result};

/// Connect to the daemon at `socket` and send `text` (connect + `write_request`).
/// Test-only: the real client (`client::send`) connects itself so it can classify
/// "no daemon" connect errors, then calls `write_request` directly.
#[cfg(test)]
pub fn send(socket: &std::path::Path, text: &str) -> Result<()> {
    let mut stream = UnixStream::connect(socket).with_context(|| format!("connect {socket:?}"))?;
    write_request(&mut stream, text)
}

/// Write one request on an already-connected stream: bytes + half-close. The
/// client connects itself (to classify "no daemon" errors), then writes here.
pub fn write_request(stream: &mut UnixStream, text: &str) -> Result<()> {
    stream.write_all(text.as_bytes()).context("write request")?;
    stream.shutdown(Shutdown::Write).context("half-close write")
}

/// Read one request to EOF from an accepted connection.
pub fn recv(stream: &mut UnixStream) -> Result<String> {
    let mut buf = String::new();
    stream.read_to_string(&mut buf).context("read request")?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    /// Fresh socket path under temp (removed if a prior run left one).
    fn scratch_sock(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koklaude-ipc-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("daemon.sock");
        let _ = std::fs::remove_file(&sock);
        sock
    }

    /// Accept one connection on `listener` and return what `recv` reads.
    fn accept_one(listener: UnixListener) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            recv(&mut stream).unwrap()
        })
    }

    #[test]
    fn round_trips_text() {
        let sock = scratch_sock("roundtrip");
        let server = accept_one(UnixListener::bind(&sock).unwrap());
        send(&sock, "hello daemon").unwrap();
        assert_eq!(server.join().unwrap(), "hello daemon");
    }

    #[test]
    fn preserves_newlines_and_unicode() {
        let sock = scratch_sock("unicode");
        let msg = "line one\nline two — café 日本語";
        let server = accept_one(UnixListener::bind(&sock).unwrap());
        send(&sock, msg).unwrap();
        assert_eq!(server.join().unwrap(), msg);
    }

    #[test]
    fn connect_to_missing_socket_errors() {
        let sock = scratch_sock("missing"); // nothing bound
        assert!(send(&sock, "x").is_err());
    }
}
