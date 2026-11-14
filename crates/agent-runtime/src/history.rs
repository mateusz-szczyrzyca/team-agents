//! The history: an append-only SQLite database of what happened — teams, rounds, runs, their steps and their changes.
//!
//! The daemon is its one writer, on a thread of its own behind a bounded queue ([`History`]); readers open the file
//! read-only ([`Reader`]) and need no daemon. The schema ([`schema`]) is built by forward-only migrations ([`migrate`])
//! and is documented, table by table, in `docs/history.md`. The records are the types of [`agent_core::history`].

mod db;
mod error;
mod handle;
pub mod migrate;
mod reader;
pub mod schema;

pub use db::{HistoryDb, Store};
pub use error::{CloseError, OpenError, ReadError, RecordKind, StartError, ValueProblem, WriteError, WriteKind};
pub use handle::{Cause, DEFAULT_QUEUE, History, Problem, Reporter, Setup, Started, TracingReporter, Writer};
pub use migrate::Migration;
pub use reader::{Reader, RoundPrompt, RunRow, Stored};
