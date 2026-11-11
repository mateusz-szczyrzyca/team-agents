//! Gate results: what the repository's gate command said about a commit.

use serde::{Deserialize, Serialize};

use super::values::{CommitId, Timestamp};
use crate::config::AbsolutePath;

/// The result of running the repository's gate on a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateResult {
    /// The commit the gate ran on: the head of the branch at that time.
    pub head: CommitId,
    /// When the gate ended.
    pub finished_at: Timestamp,
    /// What it said.
    pub outcome: GateOutcome,
    /// The file with the gate's full output.
    pub log: AbsolutePath,
}

/// How the gate ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateOutcome {
    /// The command exited with 0: the commit may be reviewed.
    Passed,
    /// The command exited with a non-zero code.
    Failed {
        /// The exit code.
        exit_code: i32,
    },
    /// The command ran longer than the configured gate timeout and was stopped.
    TimedOut,
}
