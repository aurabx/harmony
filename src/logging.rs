//! Bootstrap logging for code that runs before the real logger exists.
//!
//! The real logger is configured from the loaded config, so config loading
//! itself runs without one. Any `tracing` event emitted during the load is
//! dropped unless a subscriber is active. [`with_bootstrap_logging`] gives the
//! load a plain stderr subscriber for the duration of one call.
//!
//! The subscriber is scoped, not global. A global one would occupy the slot the
//! real logger installs into later, and the log file would never be written.

use crate::config::config::Config;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

/// Runs `f` with a stderr subscriber active on the current thread.
///
/// Events from other threads and from tasks spawned inside `f` are not
/// captured. Config loading is synchronous, so this covers it.
/// `RUST_LOG` selects the level. The default is `info`.
pub fn with_bootstrap_logging<T>(f: impl FnOnce() -> T) -> T {
    with_bootstrap_writer(std::io::stderr, f)
}

/// Same as [`with_bootstrap_logging`], writing to `writer`.
pub fn with_bootstrap_writer<W, T>(writer: W, f: impl FnOnce() -> T) -> T
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_target(false)
        .finish();
    tracing::subscriber::with_default(subscriber, f)
}

/// The values `logging.log_level` accepts. Matches `LoggingConfig::validate`.
pub const LOG_LEVELS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];

/// A `RUST_LOG` value that could not be parsed, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedRustLog {
    pub value: String,
    pub error: String,
}

/// The filter the real logger will use.
#[derive(Debug)]
pub struct LogFilter {
    pub filter: EnvFilter,
    /// Set when `RUST_LOG` was present but invalid and `log_level` was used instead.
    pub rejected_rust_log: Option<RejectedRustLog>,
}

/// Chooses the logger's filter.
///
/// A non-blank `RUST_LOG` wins. If it does not parse, the result falls back to
/// `log_level` and records the rejected value so the caller can report it.
/// Takes the environment value as an argument so it can be tested without
/// mutating process state.
///
/// Panics if `log_level` is not one of [`LOG_LEVELS`], even when `RUST_LOG`
/// wins. Config validation already enforces this for loaded files, but a
/// `Config` built in code reaches here unvalidated. `EnvFilter` would read a
/// bare word such as "warning" as a target name and silently log almost
/// nothing.
pub fn select_filter(rust_log: Option<&str>, log_level: &str) -> LogFilter {
    if !LOG_LEVELS.contains(&log_level) {
        panic!(
            "logging.log_level '{}' must be one of: {}",
            log_level,
            LOG_LEVELS.join(", ")
        );
    }

    let mut rejected_rust_log = None;
    if let Some(value) = rust_log.filter(|v| !v.trim().is_empty()) {
        match EnvFilter::try_new(value) {
            Ok(filter) => {
                return LogFilter {
                    filter,
                    rejected_rust_log: None,
                }
            }
            Err(e) => {
                rejected_rust_log = Some(RejectedRustLog {
                    value: value.to_string(),
                    error: e.to_string(),
                })
            }
        }
    }

    LogFilter {
        filter: EnvFilter::new(log_level),
        rejected_rust_log,
    }
}

/// Builds the stdout-only subscriber.
pub fn stdout_subscriber<W>(filter: EnvFilter, stdout: W) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(stdout)
        .with_file(true)
        .with_line_number(true)
        .finish()
}

/// Builds the subscriber that writes each event to both `file` and `stdout`.
///
/// The filter sits on the registry, so it applies to both layers.
pub fn file_and_stdout_subscriber<F, S>(
    filter: EnvFilter,
    file: F,
    stdout: S,
) -> impl tracing::Subscriber + Send + Sync
where
    F: for<'a> MakeWriter<'a> + Send + Sync + 'static,
    S: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let file_layer = tracing_subscriber::fmt::layer()
        .with_file(true)
        .with_line_number(true)
        .with_writer(file);
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_file(true)
        .with_line_number(true)
        .with_writer(stdout);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stdout_layer)
}

/// Installs the process-wide logger described by `config.logging`.
///
/// `RUST_LOG` selects the level when set; otherwise `logging.log_level` does.
/// Writes to stdout, and also to `logging.log_file_path` when `log_to_file`
/// is true. Uses `try_init`, so a second call is a no-op. Tests rely on that
/// because several of them start Harmony in one process.
///
/// An invalid `RUST_LOG` is reported on stderr rather than through the logger.
/// The fallback level is often `error`, which would drop a WARN event and hide
/// the problem.
///
/// Panics if the log file cannot be created. Startup cannot continue with a
/// configured log file that is not writable.
pub fn init_logging(config: &Config) {
    let logging = &config.logging;
    let rust_log = std::env::var(EnvFilter::DEFAULT_ENV).ok();
    let choice = select_filter(rust_log.as_deref(), &logging.log_level);
    if let Some(rejected) = &choice.rejected_rust_log {
        eprintln!(
            "WARN Ignoring invalid RUST_LOG '{}': {}. Using logging.log_level '{}'.",
            rejected.value, rejected.error, logging.log_level
        );
    }

    if logging.log_to_file {
        let file = std::fs::File::create(&logging.log_file_path).unwrap_or_else(|e| {
            panic!(
                "Failed to create log file '{}': {}",
                logging.log_file_path, e
            )
        });
        let _ = file_and_stdout_subscriber(choice.filter, file, std::io::stdout).try_init();
    } else {
        let _ = stdout_subscriber(choice.filter, std::io::stdout).try_init();
    }
}

/// Describes a JSON payload for a log line without any of its values.
///
/// Payloads carry patient data, so debug logs show only the top-level keys,
/// or the item count of an array, and the serialised size.
pub fn json_shape(value: &serde_json::Value) -> String {
    let size = serde_json::to_vec(value)
        .map(|bytes| format!("{} bytes", bytes.len()))
        .unwrap_or_else(|e| format!("size unknown: {}", e));
    match value {
        serde_json::Value::Object(map) => {
            let keys: Vec<&str> = map.keys().map(String::as_str).collect();
            format!("object with keys [{}], {}", keys.join(", "), size)
        }
        serde_json::Value::Array(items) => format!("array of {} items, {}", items.len(), size),
        serde_json::Value::Null => "null".to_string(),
        _ => format!("scalar, {}", size),
    }
}

/// The keys of a map, sorted, without their values.
///
/// For logging headers or DICOM query parameters, whose values can be
/// credentials or patient identifiers.
pub fn sorted_keys<V>(map: &std::collections::HashMap<String, V>) -> Vec<&str> {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

/// A URL or path with its query string and fragment removed.
///
/// FHIR search parameters such as `name` and `birthdate` are patient data.
pub fn without_query(url: &str) -> &str {
    match url.find(|c| c == '?' || c == '#') {
        Some(end) => &url[..end],
        None => url,
    }
}

/// Captures DEBUG-and-above events on the current thread, for tests that
/// check what a code path writes to the log.
#[cfg(test)]
pub(crate) mod test_capture {
    use std::cell::RefCell;
    use std::io;
    use std::sync::{Arc, Mutex, Once};
    use tracing::subscriber::Interest;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

    thread_local! {
        /// The log that events on this thread go to, while a capture is active.
        static ACTIVE: RefCell<Option<CapturedLog>> = const { RefCell::new(None) };
    }

    /// `try_with` because an event can fire while thread-locals are being
    /// destroyed. Such an event is not captured.
    fn active_log() -> Option<CapturedLog> {
        ACTIVE.try_with(|slot| slot.borrow().clone()).ok().flatten()
    }

    #[derive(Clone, Default)]
    pub(crate) struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl CapturedLog {
        pub(crate) fn contents(&self) -> String {
            String::from_utf8(self.0.lock().expect("log buffer lock").clone()).expect("utf-8 log")
        }
    }

    impl io::Write for CapturedLog {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("log buffer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Writes to the active log, or discards when this thread has none.
    struct ActiveLogSink(Option<CapturedLog>);

    impl io::Write for ActiveLogSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match &mut self.0 {
                Some(log) => log.write(buf),
                None => Ok(buf.len()),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Writes each formatted event to the emitting thread's active log.
    struct ActiveLogSinkWriter;

    impl<'a> MakeWriter<'a> for ActiveLogSinkWriter {
        type Writer = ActiveLogSink;

        fn make_writer(&'a self) -> Self::Writer {
            ActiveLogSink(active_log())
        }
    }

    /// Enables an event only on a thread with an active capture.
    ///
    /// `filter_fn` is not usable here: it caches its answer per callsite, so
    /// a callsite first hit on a thread without a capture would stay off.
    struct ActiveCaptureFilter;

    impl<S: tracing::Subscriber> Layer<S> for ActiveCaptureFilter {
        fn register_callsite(&self, _: &'static tracing::Metadata<'static>) -> Interest {
            Interest::sometimes()
        }

        fn enabled(&self, _: &tracing::Metadata<'_>, _: Context<'_, S>) -> bool {
            active_log().is_some()
        }
    }

    /// Installs, once per test process, a global DEBUG subscriber that only
    /// records events from threads with an active capture.
    ///
    /// A per-thread `set_default` subscriber raced with tests that log while
    /// no subscriber exists: those can cache a callsite as never enabled, and
    /// the capture then misses the event. The global subscriber registers
    /// every callsite as "sometimes", so that cache entry can never form.
    ///
    /// Panics if another global subscriber is already installed, because the
    /// captures would then record nothing.
    fn install_global_subscriber() {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(ActiveLogSinkWriter)
                .finish()
                .with(ActiveCaptureFilter);
            tracing::subscriber::set_global_default(subscriber)
                .expect("test_capture needs the global subscriber, but one is already set");
        });
    }

    /// Ends a capture when dropped, restoring any capture it replaced.
    pub(crate) struct CaptureGuard {
        previous: Option<CapturedLog>,
    }

    impl Drop for CaptureGuard {
        fn drop(&mut self) {
            let previous = self.previous.take();
            ACTIVE.with(|slot| *slot.borrow_mut() = previous);
        }
    }

    /// Captures DEBUG-and-above events on the current thread until the guard
    /// drops. `#[tokio::test]` runs on one thread, so this also covers
    /// awaited code. Events from other threads, such as `spawn_blocking`,
    /// are not captured.
    pub(crate) fn capture_debug() -> (CapturedLog, CaptureGuard) {
        install_global_subscriber();
        let log = CapturedLog::default();
        let previous = ACTIVE.with(|slot| slot.borrow_mut().replace(log.clone()));
        (log, CaptureGuard { previous })
    }

    mod tests {
        use super::capture_debug;

        #[test]
        fn captures_only_the_current_thread() {
            let (log, _guard) = capture_debug();
            std::thread::spawn(|| tracing::debug!("OTHER-THREAD-MARKER"))
                .join()
                .expect("join");
            tracing::debug!("this-thread-shape");

            let text = log.contents();
            assert!(text.contains("this-thread-shape"), "log: {text}");
            assert!(!text.contains("OTHER-THREAD-MARKER"), "log: {text}");
        }

        fn shared_callsite() {
            tracing::debug!("shared-callsite-shape");
        }

        /// Other tests often reach a callsite first, on a thread with no
        /// capture. That must not switch the callsite off for later captures.
        #[test]
        fn callsite_first_hit_without_capture_is_still_captured() {
            drop(capture_debug());
            std::thread::spawn(shared_callsite).join().expect("join");

            let (log, _guard) = capture_debug();
            shared_callsite();

            let text = log.contents();
            assert!(text.contains("shared-callsite-shape"), "log: {text}");
        }

        #[test]
        fn nothing_is_captured_after_the_guard_drops() {
            let (log, guard) = capture_debug();
            tracing::debug!("before-drop-shape");
            drop(guard);
            tracing::debug!("AFTER-DROP-MARKER");

            let text = log.contents();
            assert!(text.contains("before-drop-shape"), "log: {text}");
            assert!(!text.contains("AFTER-DROP-MARKER"), "log: {text}");
        }
    }
}
