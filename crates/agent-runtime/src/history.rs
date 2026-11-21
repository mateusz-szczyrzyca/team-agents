//! The history: an append-only SQLite database of what happened — teams, rounds, runs, their steps and their changes.
//!
//! The daemon is its one writer, on a thread of its own behind a bounded queue ([`History`]); readers open the file
//! read-only ([`Reader`]) and need no daemon. The schema ([`schema`]) is built by forward-only migrations ([`migrate`])
//! and is documented, table by table, in `docs/history.md`. The records are the types of [`agent_core::history`].

mod codec;
mod db;
mod error;
mod handle;
pub mod judgements;
mod ledger;
pub mod log;
pub mod migrate;
mod notes;
mod plans;
mod reader;
mod redact;
mod retention;
pub mod schema;
mod write;

pub use db::{HistoryDb, Store};
pub use error::{CloseError, OpenError, ReadError, RecordKind, StartError, ValueProblem, WriteError, WriteKind};
pub use handle::{
    CaptureKind, Cause, DEFAULT_QUEUE, History, Problem, Reporter, Setup, Started, TracingReporter, Writer,
};
pub use judgements::JudgementRecord;
pub use log::{HistoryLayer, Scoped, ensure_scoped_callsites, install_panic_hook, log_decision, log_outside_call};
pub use migrate::Migration;
pub use notes::Noting;
pub use plans::{PlanRead, PlanRecord};
pub use reader::{Reader, RoundPrompt, RunRow, Stored};

pub use reader::query::{LogFilter, LogWindow};

pub use retention::{RetainError, Retention};
