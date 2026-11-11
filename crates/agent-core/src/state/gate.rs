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
///
/// Read through a private wire enum that refuses a field the variant does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", from = "GateOutcomeWire")]
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

/// What a [`GateOutcome`] is read as.
///
/// serde skips an unknown field next to the tag of a unit variant of an internally tagged enum, even with
/// `deny_unknown_fields`; a variant with a (here empty) struct body does refuse it.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GateOutcomeWire {
    /// See [`GateOutcome::Passed`].
    Passed {},
    /// See [`GateOutcome::Failed`].
    Failed {
        /// See [`GateOutcome::Failed`].
        exit_code: i32,
    },
    /// See [`GateOutcome::TimedOut`].
    TimedOut {},
}

impl From<GateOutcomeWire> for GateOutcome {
    fn from(wire: GateOutcomeWire) -> Self {
        match wire {
            GateOutcomeWire::Passed {} => Self::Passed,
            GateOutcomeWire::Failed { exit_code } => Self::Failed { exit_code },
            GateOutcomeWire::TimedOut {} => Self::TimedOut,
        }
    }
}

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
