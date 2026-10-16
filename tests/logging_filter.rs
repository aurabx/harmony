//! The real logger must honour `RUST_LOG`, then `logging.log_level`.
//!
//! These tests build the same subscribers `init_logging` installs, but with
//! buffer writers and a scoped default, so they never touch the global slot.

use harmony::logging::{file_and_stdout_subscriber, select_filter, stdout_subscriber};
use std::io;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

/// Collects everything the subscriber writes so tests can assert on it.
#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    fn contents(&self) -> String {
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

impl<'a> MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn emit_one_of_each_level() {
    tracing::debug!("debug-marker");
    tracing::info!("info-marker");
    tracing::error!("error-marker");
}

/// Runs the stdout-only subscriber for the given inputs and returns its output.
fn stdout_output(rust_log: Option<&str>, log_level: &str) -> String {
    let log = CapturedLog::default();
    let choice = select_filter(rust_log, log_level);
    let subscriber = stdout_subscriber(choice.filter, log.clone());
    tracing::subscriber::with_default(subscriber, emit_one_of_each_level);
    log.contents()
}

#[test]
fn log_level_applies_when_rust_log_is_unset() {
    let out = stdout_output(None, "error");
    assert!(out.contains("error-marker"), "output: {out}");
    assert!(!out.contains("info-marker"), "output: {out}");
    assert!(!out.contains("debug-marker"), "output: {out}");
}

#[test]
fn log_level_debug_lets_debug_through() {
    let out = stdout_output(None, "debug");
    assert!(out.contains("debug-marker"), "output: {out}");
}

#[test]
fn rust_log_overrides_log_level() {
    let out = stdout_output(Some("debug"), "error");
    assert!(out.contains("debug-marker"), "output: {out}");

    let out = stdout_output(Some("error"), "debug");
    assert!(!out.contains("debug-marker"), "output: {out}");
    assert!(!out.contains("info-marker"), "output: {out}");
}

#[test]
fn empty_rust_log_is_treated_as_unset() {
    let choice = select_filter(Some("  "), "error");
    assert!(choice.rejected_rust_log.is_none());

    let out = stdout_output(Some(""), "error");
    assert!(out.contains("error-marker"), "output: {out}");
    assert!(!out.contains("info-marker"), "output: {out}");
}

#[test]
fn invalid_rust_log_falls_back_to_log_level_and_is_reported() {
    let choice = select_filter(Some("harmony=notalevel"), "error");
    let rejected = choice
        .rejected_rust_log
        .as_ref()
        .expect("invalid RUST_LOG must be reported");
    assert_eq!(rejected.value, "harmony=notalevel");
    assert!(!rejected.error.is_empty());

    let out = stdout_output(Some("harmony=notalevel"), "error");
    assert!(out.contains("error-marker"), "output: {out}");
    assert!(!out.contains("info-marker"), "output: {out}");
}

#[test]
fn valid_rust_log_is_not_reported() {
    let choice = select_filter(Some("harmony=debug,info"), "error");
    assert!(choice.rejected_rust_log.is_none());
}

#[test]
fn file_mode_filters_both_file_and_stdout() {
    let file = CapturedLog::default();
    let stdout = CapturedLog::default();
    let choice = select_filter(None, "error");
    let subscriber = file_and_stdout_subscriber(choice.filter, file.clone(), stdout.clone());
    tracing::subscriber::with_default(subscriber, emit_one_of_each_level);

    for (name, out) in [("file", file.contents()), ("stdout", stdout.contents())] {
        assert!(out.contains("error-marker"), "{name} output: {out}");
        assert!(!out.contains("info-marker"), "{name} output: {out}");
        assert!(!out.contains("debug-marker"), "{name} output: {out}");
    }
}

/// A bare word such as "warning" parses as a target directive, which would
/// silently turn off almost all logging. It must be rejected instead.
#[test]
#[should_panic(expected = "logging.log_level 'warning'")]
fn log_level_that_is_not_a_level_name_panics() {
    select_filter(None, "warning");
}

#[test]
#[should_panic(expected = "logging.log_level 'harmony=debug'")]
fn log_level_directive_is_rejected() {
    select_filter(None, "harmony=debug");
}

/// A valid RUST_LOG must not hide a broken log_level, because the process
/// would start fine and then log nothing once RUST_LOG is removed.
#[test]
#[should_panic(expected = "logging.log_level 'warning'")]
fn invalid_log_level_is_rejected_even_when_rust_log_is_valid() {
    select_filter(Some("debug"), "warning");
}
