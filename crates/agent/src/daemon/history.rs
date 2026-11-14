//! The daemon's side of the history: opening it from `[history]` of the configuration, and what the owner records in it.
//!
//! The history never stops or delays the work (`docs/history.md`): when the database cannot be opened the daemon says so
//! once and runs without it, and every record that is handed in after that is counted as lost.

use std::sync::Arc;

use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Setup, TracingReporter, Writer};
use agent_runtime::process::error_chain;
use agent_runtime::settings::Settings;
use agent_runtime::store::Home;

/// The history of a daemon, and the thread that writes it.
pub(super) struct Opened {
    /// The handle everything records through.
    pub(super) history: History,
    /// The writer, to close when the daemon ends; `None` when nothing is written.
    pub(super) writer: Option<Writer>,
}

/// Opens the history of `home` as `settings` say: off, written, or unavailable (the database cannot be used).
pub(super) async fn open(settings: &Settings, home: &Home) -> Opened {
    let config = settings.config.history();
    if !config.enabled {
        return Opened {
            history: History::disabled(),
            writer: None,
        };
    }
    let reporter = Arc::new(TracingReporter);
    let path = home.history_file();
    let limit = config.max_text_bytes;
    // Opening makes the file, switches it to write-ahead logging and migrates it: blocking work, off the runtime.
    let opened = tokio::task::spawn_blocking(move || HistoryDb::open(&path, limit)).await;
    let database = match opened {
        Ok(Ok(database)) => database,
        Ok(Err(error)) => {
            return Opened {
                history: History::unavailable(&error, reporter),
                writer: None,
            };
        }
        Err(error) => {
            tracing::error!(error = %error_chain(&error), "the task that opens the history failed: the daemon runs without it");
            return Opened {
                history: History::disabled(),
                writer: None,
            };
        }
    };
    let setup = Setup {
        reporter: reporter.clone(),
        capacity: DEFAULT_QUEUE,
    };
    match History::start(database, setup) {
        Ok(started) => Opened {
            history: started.history,
            writer: Some(started.writer),
        },
        Err(error) => {
            tracing::error!(error = %error_chain(&error), "the writer of the history cannot be started: the daemon runs without it");
            Opened {
                history: History::disabled(),
                writer: None,
            }
        }
    }
}
