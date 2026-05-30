//! Wire protocol between the hook client and the daemon over a unix socket.
//!
//! Frame contract: **one connection = one request**. The client writes the
//! UTF-8 text and half-closes its write half; the daemon reads to EOF. No
//! length prefix, no delimiter — EOF *is* the frame boundary (one request per
//! connection, so there's nothing to disambiguate). Fire-and-forget: no reply.

// Used by the daemon + client (4b/4d) — remove this allow when wired.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::{Context, Result};

/// Send `text` to the daemon at `socket`: connect, write, half-close.
pub fn send(socket: &Path, text: &str) -> Result<()> {
    let mut stream = UnixStream::connect(socket).with_context(|| format!("connect {socket:?}"))?;
    stream.write_all(text.as_bytes()).context("write request")?;
    stream
        .shutdown(Shutdown::Write)
        .context("half-close write")?;
    Ok(())
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
