//! Diagnostics emitted while loading config must reach the operator (AURA-2436).
//!
//! The binary loads config before the real logger exists. These tests cover
//! the bootstrap logger that wraps the load, and the rule that a pipeline file
//! which cannot be parsed fails the load instead of being skipped.

use harmony::config::config::{Config, ConfigError};
use harmony::logging::with_bootstrap_writer;
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

const MAIN_CONFIG: &str = r#"
[proxy]
id = "load-diagnostics-test"

[management]
enabled = false

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080

[pipelines.core]
networks = ["default"]
endpoints = ["http_in"]
backends = ["api_backend"]

[endpoints.http_in]
service = "http"
[endpoints.http_in.options]
path_prefix = "/api"

[backends.api_backend]
service = "http"
[backends.api_backend.options]
host = "127.0.0.1"
port = 9

[services.http]
module = ""
"#;

/// Writes `main_toml` plus the given `(file name, contents)` pairs under
/// `pipelines/` and returns the temp dir with the main config path.
fn write_config(main_toml: &str, pipeline_files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, main_toml).expect("write config");
    if !pipeline_files.is_empty() {
        let pipelines_dir = dir.path().join("pipelines");
        std::fs::create_dir(&pipelines_dir).expect("create pipelines dir");
        for (name, contents) in pipeline_files {
            std::fs::write(pipelines_dir.join(name), contents).expect("write pipeline file");
        }
    }
    let path = path.to_str().expect("utf-8 path").to_string();
    (dir, path)
}

#[test]
fn bootstrap_logger_captures_events_emitted_by_the_closure() {
    let log = CapturedLog::default();

    let value = with_bootstrap_writer(log.clone(), || {
        tracing::warn!("bootstrap probe warning");
        42
    });

    assert_eq!(value, 42);
    assert!(
        log.contents().contains("bootstrap probe warning"),
        "warning was not captured: {:?}",
        log.contents()
    );
}

#[test]
fn bootstrap_logger_does_not_block_a_later_global_subscriber() {
    let log = CapturedLog::default();
    with_bootstrap_writer(log, || tracing::warn!("emitted during bootstrap"));

    // The real logger is installed globally after config load. A scoped
    // bootstrap subscriber must leave that slot free.
    let installed = tracing_subscriber::fmt().try_init();
    assert!(
        installed.is_ok(),
        "global subscriber could not be installed: {:?}",
        installed.err()
    );
}

#[test]
fn unresolved_backend_target_is_reported_during_load() {
    let main = format!(
        "{}\n[backends.orphan]\nservice = \"http\"\ntarget_ref = \"missing_target\"\n",
        MAIN_CONFIG
    );
    let (_dir, path) = write_config(&main, &[]);
    let log = CapturedLog::default();

    let result = with_bootstrap_writer(log.clone(), || Config::load(&path));

    assert!(result.is_ok(), "load failed: {:?}", result.err());
    let output = log.contents();
    assert!(
        output.contains("orphan") && output.contains("missing_target"),
        "expected a warning naming the backend and the target, got: {:?}",
        output
    );
}

#[test]
fn unparseable_pipeline_file_fails_the_load() {
    let (_dir, path) = write_config(MAIN_CONFIG, &[("broken.toml", "[pipelines.broken\n")]);

    let err = Config::load(&path).expect_err("load must fail on an unparseable pipeline file");

    match err {
        ConfigError::InvalidProxy { reason, .. } => {
            assert!(
                reason.contains("broken.toml"),
                "error must name the file, got: {}",
                reason
            );
        }
        other => panic!("unexpected error variant: {:?}", other),
    }
}

#[test]
fn valid_pipeline_file_still_loads() {
    let pipeline = r#"
[pipelines.extra]
networks = ["default"]
endpoints = ["extra_in"]
backends = ["api_backend"]

[endpoints.extra_in]
service = "http"
[endpoints.extra_in.options]
path_prefix = "/extra"
"#;
    let (_dir, path) = write_config(MAIN_CONFIG, &[("extra.toml", pipeline)]);

    let config = Config::load(&path).expect("valid pipeline file must load");

    assert!(config.pipelines.contains_key("extra"));
}
