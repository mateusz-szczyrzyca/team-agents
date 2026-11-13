//! Gate results: what the repository's gate command said about a commit.

use std::num::NonZeroI32;

use serde::{Deserialize, Serialize};

use super::strict::strict_enum;
use super::values::{CommitId, Timestamp};
use crate::config::AbsolutePath;
use crate::kinds::{Keyword, UnknownKeyword};

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

/// Whether the worktree held changes when the gate ran.
///
/// A gate that passes on a tree with changes says nothing about the commit: the changes were part of what ran.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<GateTree>)]
pub enum GateTree {
    /// Nothing was changed or untracked, and no merge was under way.
    Clean,
    /// Something was changed or untracked, or a merge was waiting to be committed.
    NotClean,
}

impl Keyword for GateTree {
    const WHAT: &'static str = "gate tree";
}

impl TryFrom<String> for GateTree {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// What the daemon recorded about one run of the gate: its result and the state of the tree it ran on.
///
/// The record is the only thing that may be trusted about a commit's gate: an agent's claim is not one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateRecord {
    /// Whether the tree was clean.
    pub tree: GateTree,
    /// What the gate said about `result.head`.
    pub result: GateResult,
}

impl GateRecord {
    /// Whether the gate passed on a clean tree: a pass of the commit itself.
    #[must_use]
    pub fn counts_as_pass(&self) -> bool {
        false
    }
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

    const HEAD: &str = "26b65de7e4076596768ca96b48a7f94799c93874";

    fn record(tree: GateTree, outcome: GateOutcome) -> GateRecord {
        GateRecord {
            tree,
            result: GateResult {
                head: HEAD.parse().unwrap(),
                finished_at: "2026-11-13T09:30:00Z".parse().unwrap(),
                outcome,
                log: "/home/u/agents/tasks/task-1/gate-1.log".parse().unwrap(),
            },
        }
    }

    fn failed(code: i32) -> GateOutcome {
        GateOutcome::Failed {
            exit_code: NonZeroI32::new(code).unwrap(),
        }
    }

    #[test]
    fn a_record_counts_as_a_pass_only_when_the_gate_passed_on_a_clean_tree() {
        let cases = [
            (GateTree::Clean, GateOutcome::Passed, true),
            (GateTree::NotClean, GateOutcome::Passed, false),
            (GateTree::Clean, failed(2), false),
            (GateTree::NotClean, failed(2), false),
            (GateTree::Clean, GateOutcome::TimedOut, false),
            (GateTree::NotClean, GateOutcome::TimedOut, false),
        ];

        for (tree, outcome, expected) in cases {
            assert_eq!(record(tree, outcome).counts_as_pass(), expected, "{tree} {outcome:?}");
        }
    }

    #[test]
    fn a_record_is_written_with_its_tree_and_its_result_and_read_back_equal() {
        let original = record(GateTree::NotClean, GateOutcome::Passed);

        let value = serde_json::to_value(&original).unwrap();

        assert_eq!(
            value,
            json!({
                "tree": "not_clean",
                "result": {
                    "head": HEAD,
                    "finished_at": "2026-11-13T09:30:00Z",
                    "outcome": {"kind": "passed"},
                    "log": "/home/u/agents/tasks/task-1/gate-1.log"
                }
            })
        );
        assert_eq!(serde_json::from_value::<GateRecord>(value).unwrap(), original);
    }

    #[test]
    fn a_record_with_an_unknown_tree_or_field_is_refused_when_read() {
        let good = serde_json::to_value(record(GateTree::Clean, GateOutcome::Passed)).unwrap();
        let mut unknown_tree = good.clone();
        unknown_tree["tree"] = json!("tidy");
        let mut extra = good.clone();
        extra["extra"] = json!(1);
        let mut missing = good;
        missing.as_object_mut().unwrap().remove("tree");

        for bad in [unknown_tree, extra, missing] {
            assert_refused::<GateRecord>(&bad);
        }
    }
}
