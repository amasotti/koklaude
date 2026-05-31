//! Persistent JSON logging for every koklaude process.
//!
//! All processes append one JSON object per line to a single daily-rotated file
//! under `~/.koklaude/logs/` (see docs/logging.md). The per-event `target` (the
//! module path, e.g. `koklaude::daemon`) identifies the component — free and
//! thread-correct, unlike a process-wide span. The hook wraps its work in a
//! `session_id` span so its events are attributable to a Claude session.
//!
//! Best-effort: any setup failure is swallowed. Logging must never break the
//! hook or daemon ("never block Claude Code").

use std::path::PathBuf;

use tracing::Subscriber;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

/// Override the log directory (tests), mirroring `config`'s `KOKLAUDE_HOME`.
const LOG_DIR_ENV: &str = "KOKLAUDE_LOG_DIR";
const FILE_PREFIX: &str = "koklaude";
const FILE_SUFFIX: &str = "jsonl";

/// Initialise process-wide JSON logging to the daily file. Best-effort: a failed
/// dir/appender/already-set is swallowed — worst case is no logs, never a crash.
pub fn init() {
    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(appender) = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILE_PREFIX)
        .filename_suffix(FILE_SUFFIX)
        .build(&dir)
    else {
        return;
    };
    let _ = tracing::subscriber::set_global_default(subscriber(appender));
}

/// Build the JSON subscriber over `writer`. Separate from [`init`] so tests can
/// drive it with `with_default` + a capturing writer (the global default can be
/// set only once per process).
fn subscriber<W>(writer: W) -> impl Subscriber
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(writer)
        .with_current_span(true) // carry the hook's session_id span
        .with_span_list(false)
        .with_filter(filter_fn(|meta| {
            let t = meta.target();
            t.starts_with("koklaude") || t.starts_with("hanasu")
        }));
    tracing_subscriber::registry().with(layer)
}

/// `$KOKLAUDE_LOG_DIR` if set, else `~/.koklaude/logs`.
fn log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(LOG_DIR_ENV) {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".koklaude/logs")
}

/// Test-only capture of the JSON subscriber, shared with other modules' tests
/// (the global default can be set only once per process, so they capture via a
/// scoped `with_default` instead).
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    /// A `MakeWriter` that captures everything written into a shared buffer.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;
        fn make_writer(&'a self) -> Buffer {
            self.clone()
        }
    }

    /// Run `f` with the JSON subscriber active and return everything it logged.
    pub(crate) fn capture(f: impl FnOnce()) -> String {
        let buf = Buffer::default();
        tracing::subscriber::with_default(subscriber(buf.clone()), f);
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::capture;
    use tracing::{info, info_span};

    // Probe the real JSON shape — assert against what tracing-subscriber actually
    // emits, not a guessed schema.
    #[test]
    fn emits_one_json_object_with_level_timestamp_and_fields() {
        let line = capture(|| info!(chars = 214, synth_ms = 92, "spoke"));
        let v: serde_json::Value =
            serde_json::from_str(line.trim()).expect("one valid JSON object per line");
        assert_eq!(v["level"], "INFO");
        assert!(v["timestamp"].is_string());
        assert_eq!(v["fields"]["message"], "spoke");
        assert_eq!(v["fields"]["chars"], 214);
        assert_eq!(v["fields"]["synth_ms"], 92);
    }

    #[test]
    fn event_carries_the_session_span_field() {
        let line = capture(|| {
            let span = info_span!("turn", session_id = "abc123");
            let _g = span.enter();
            info!("hook fired");
        });
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["span"]["session_id"], "abc123");
    }

    #[test]
    fn drops_dependency_targets() {
        // `ort` and friends log through the same facade; only our events survive.
        let line = capture(|| {
            info!(target: "ort::logging", "noise");
            info!(target: "koklaude::daemon", "kept");
        });
        assert!(line.contains("kept"));
        assert!(!line.contains("noise"));
    }

    #[test]
    fn default_log_dir_is_under_dot_koklaude() {
        if std::env::var(LOG_DIR_ENV).is_err() {
            assert!(log_dir().ends_with(".koklaude/logs"));
        }
    }

    // Exercise the real RollingFileAppender (not the buffer): proves the on-disk
    // filename pattern `koklaude.YYYY-MM-DD.jsonl` and that a line is flushed.
    #[test]
    fn writes_to_a_real_daily_file() {
        let dir = std::env::temp_dir().join("koklaude-logtest-real");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let appender = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix(FILE_PREFIX)
            .filename_suffix(FILE_SUFFIX)
            .build(&dir)
            .unwrap();
        tracing::subscriber::with_default(subscriber(appender), || {
            info!(chars = 5, "spoke");
        });

        let file = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.starts_with("koklaude.") && n.ends_with(".jsonl")
            })
            .expect("a koklaude.<date>.jsonl file");

        let v: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
        assert_eq!(v["fields"]["message"], "spoke");
        assert_eq!(v["fields"]["chars"], 5);
    }
}
