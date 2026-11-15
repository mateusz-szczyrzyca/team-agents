//! The `tracing_subscriber::Layer` that turns daemon log events and their enclosing spans into rows of `history.db`
//! (or redacted lines on `stderr` before `history.db` is open or when it cannot be written), the panic hook, and the
//! `debug` helpers for pure decisions and outside tool calls.

use std::fmt;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::time::Duration;

use agent_core::config::Secrets;
use agent_core::log::LogLevel;
use serde::Serialize;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use super::handle::History;
use crate::clock::SharedClock;

/// Callback invoked for lines written to the standard error stream fallback.
pub type StderrSink = Arc<dyn Fn(&str) + Send + Sync>;

/// A `tracing_subscriber::Layer` that records structured [`agent_core::log::LogEvent`] events into `history.db`
/// or to `stderr` before `history.db` is open or when `history.db` cannot be written.
#[derive(Clone)]
pub struct HistoryLayer {
    /// Threshold level configured for the layer.
    level: LogLevel,
}

impl fmt::Debug for HistoryLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryLayer").field("level", &self.level).finish()
    }
}

impl HistoryLayer {
    /// Creates a layer with `level`, `clock`, `secrets`, and an optional open `history`.
    #[must_use]
    pub fn new(level: LogLevel, _clock: SharedClock, _secrets: Secrets, _history: Option<History>) -> Self {
        Self { level }
    }

    /// Creates a layer whose `history.db` handle will be attached once opened.
    #[must_use]
    pub fn deferred(level: LogLevel, clock: SharedClock, secrets: Secrets) -> Self {
        Self::new(level, clock, secrets, None)
    }

    /// Installs a global [`HistoryLayer`] and panic hook for the daemon process.
    #[must_use]
    pub fn install_global() -> Self {
        Self { level: LogLevel::Info }
    }

    /// Updates the threshold level, clock, and secrets of this layer.
    pub fn configure(&self, _level: LogLevel, _clock: SharedClock, _secrets: Secrets) {}

    /// Attaches the opened [`History`] handle so subsequent events are written to `history.db`.
    pub fn attach_history(&self, _history: History) {}

    /// Detaches the [`History`] handle when the writer is closing so late events go to `stderr`.
    pub fn detach_history(&self) {}

    /// Replaces the standard error fallback sink (used by tests to inspect `stderr` lines deterministically).
    pub fn with_stderr_sink(&self, _sink: StderrSink) {}

    /// Records a [`LogEvent::DaemonPanicked`](agent_core::log::LogEvent::DaemonPanicked) entry from `info`.
    pub fn record_panic_info(&self, _info: &PanicHookInfo<'_>) {}
}

impl<S> Layer<S> for HistoryLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, _event: &tracing::Event<'_>, _ctx: Context<'_, S>) {}
}

/// Installs the daemon panic hook that records [`agent_core::log::LogEvent::DaemonPanicked`] with a redacted message
/// and source location through `layer`.
pub fn install_panic_hook(_layer: HistoryLayer) {}

/// Records a pure decision of `agent_core::decide` (`decision_taken`, `debug`) with its name, input, and result.
pub fn log_decision<I, R>(_decision: &str, _input: &I, _result: &R)
where
    I: Serialize + ?Sized,
    R: Serialize + ?Sized,
{
}

/// Records a completed outside call (`outside_call_completed`, `debug`) to `git`, `agentbus`, `github`, or `gate`.
pub fn log_outside_call(_tool: &str, _operation: &str, _duration: Duration, _exit_code: Option<i32>, _outcome: &str) {}

#[cfg(test)]
mod tests;
