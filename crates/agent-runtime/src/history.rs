//! The history: an append-only SQLite database of what happened — teams, rounds, runs, their steps and their changes.
//!
//! The daemon is its one writer, on a thread of its own behind a bounded queue ([`History`]); readers open the file
//! read-only ([`Reader`]) and need no daemon. The schema ([`schema`]) is built by forward-only migrations ([`migrate`])
//! and is documented, table by table, in `docs/history.md`. The records are the types of [`agent_core::history`].

mod codec;
mod db;
mod error;
mod handle;
mod ledger;
pub mod migrate;
mod notes;
mod reader;
mod redact;
pub mod schema;
mod write;

pub use db::{HistoryDb, Store};
pub use error::{CloseError, OpenError, ReadError, RecordKind, StartError, ValueProblem, WriteError, WriteKind};
pub use handle::{
    CaptureKind, Cause, DEFAULT_QUEUE, History, Problem, Reporter, Setup, Started, TracingReporter, Writer,
};
pub use migrate::Migration;
pub use notes::Noting;
pub use reader::{Reader, RoundPrompt, RunRow, Stored};
