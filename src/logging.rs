//! Bootstrap logging for code that runs before the real logger exists.
//!
//! The real logger is configured from the loaded config, so config loading
//! itself runs without one. Any `tracing` event emitted during the load is
//! dropped unless a subscriber is active. [`with_bootstrap_logging`] gives the
//! load a plain stderr subscriber for the duration of one call.
//!
//! The subscriber is scoped, not global. A global one would occupy the slot the
//! real logger installs into later, and the log file would never be written.

use tracing_subscriber::fmt::MakeWriter;
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
