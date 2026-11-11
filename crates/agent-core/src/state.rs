//! The persistent state of the harness: teams, their members and roles, the task list with the progress of each
//! item, runs, questions, pull requests, reviews and gate results (see `docs/state.md`).
//!
//! The daemon owns one [`State`], saves it as JSON and reads it back; the CLI and the dashboard see parts of it
//! through the control protocol ([`crate::protocol`]). Everything here is pure data with its rules: a value that
//! exists is a valid one, and a text that is not valid is refused when it is read.

mod values;

pub use values::{CommitId, InvalidCommitId, InvalidTimestamp, PrNumber, SessionId, Timestamp};
