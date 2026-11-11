//! Gate results: what the repository's gate command said about a commit.

use std::num::NonZeroI32;

use serde::{Deserialize, Serialize};

use super::strict::strict_enum;
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
///
/// Deserialization refuses fields that the variant does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateOutcome {
    /// The command exited with 0: the commit may be reviewed.
    Passed,
    /// The command exited with a non-zero code.
    Failed {
        /// The exit code; never 0, which is a pass.
        exit_code: NonZeroI32,
    },
    /// The command ran longer than the configured gate timeout and was stopped.
    TimedOut,
}

/// How a [`GateOutcome`] is read; see [`strict_enum!`].
#[derive(Deserialize)]
#[serde(remote = "GateOutcome", tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GateOutcomeWire {
    /// See [`GateOutcome::Passed`].
    Passed,
    /// See [`GateOutcome::Failed`].
    Failed {
        /// See [`GateOutcome::Failed`].
        exit_code: NonZeroI32,
    },
    /// See [`GateOutcome::TimedOut`].
    TimedOut,
}

strict_enum!(GateOutcome, GateOutcomeWire);

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::fixtures::assert_refused;

    #[test]
    fn a_failed_gate_with_exit_code_zero_is_refused_when_read() {
        assert_refused::<GateOutcome>(&json!({"kind": "failed", "exit_code": 0}));
    }

    #[test]
    fn a_failed_gate_keeps_any_other_exit_code_the_way_it_was_written() {
        for code in [1, 2, 127, -1, i32::MIN, i32::MAX] {
            let value = json!({"kind": "failed", "exit_code": code});

            let outcome: GateOutcome = serde_json::from_value(value.clone()).unwrap();

            assert_eq!(serde_json::to_value(outcome).unwrap(), value);
        }
    }

    #[test]
    fn a_failed_gate_without_an_exit_code_is_refused_when_read() {
        assert_refused::<GateOutcome>(&json!({"kind": "failed"}));
    }
}
