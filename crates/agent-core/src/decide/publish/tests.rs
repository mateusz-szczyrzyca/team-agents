use std::num::NonZeroI32;
use std::str::FromStr;

use super::*;
use crate::state::{GateOutcome, GateResult, GateTree, Timestamp};

fn commit(digit: char) -> CommitId {
    CommitId::from_str(&digit.to_string().repeat(40)).unwrap()
}

fn record(head: char, outcome: GateOutcome, tree: GateTree) -> GateRecord {
    GateRecord {
        tree,
        result: GateResult {
            head: commit(head),
            finished_at: Timestamp::from_str("2026-11-16T10:00:00Z").unwrap(),
            outcome,
            log: AbsolutePath::from_str("/home/tasks/t/gate-1.log").unwrap(),
        },
    }
}

fn failed() -> GateOutcome {
    GateOutcome::Failed {
        exit_code: NonZeroI32::new(2).unwrap(),
    }
}

/// A lead that is done, with work of its own, and what is given.
fn done<'a>(tip: &'a CommitId, record: Option<&'a GateRecord>, gated_for_tip: bool) -> Waiting<'a> {
    Waiting {
        tip,
        lead_done: true,
        own_work: true,
        record,
        harness_gate: if gated_for_tip {
            HarnessGate::Ran
        } else {
            HarnessGate::NotRun
        },
    }
}

// Rule: a pass at the tip on a clean tree publishes, whoever ran the gate and however the lead ended.
#[test]
fn a_pass_at_the_tip_on_a_clean_tree_publishes() {
    let tip = commit('a');
    let passed = record('a', GateOutcome::Passed, GateTree::Clean);

    for lead_done in [true, false] {
        let waiting = Waiting {
            lead_done,
            ..done(&tip, Some(&passed), false)
        };
        assert_eq!(gate_step(&waiting), GateStep::Publish, "lead_done={lead_done}");
    }
}

// Rule: a lead that is done and has no gate of its tip gets the gate run by the harness.
#[test]
fn a_done_lead_without_a_gate_of_its_tip_gets_the_gate_run() {
    let tip = commit('b');
    let older = record('a', GateOutcome::Passed, GateTree::Clean);

    assert_eq!(gate_step(&done(&tip, None, false)), GateStep::Run);
    assert_eq!(gate_step(&done(&tip, Some(&older), false)), GateStep::Run);
}

// Rule: a lead that sleeps is not done: the harness neither runs the gate nor sends it back.
#[test]
fn a_lead_that_is_not_done_waits() {
    let tip = commit('b');
    let older = record('a', GateOutcome::Passed, GateTree::Clean);
    let red = record('b', failed(), GateTree::Clean);

    for (record, own_work) in [(None, true), (Some(&older), true), (Some(&red), true), (None, false)] {
        let waiting = Waiting {
            tip: &tip,
            lead_done: false,
            own_work,
            record,
            harness_gate: HarnessGate::NotRun,
        };
        assert_eq!(gate_step(&waiting), GateStep::Wait, "{record:?}");
    }
}

// Rule: a gate that failed or timed out at the tip is run once more by the harness before the lead is sent back,
// whoever ran the one that failed.
#[test]
fn a_gate_that_failed_at_the_tip_is_run_once_more_by_the_harness() {
    let tip = commit('a');

    for outcome in [failed(), GateOutcome::TimedOut] {
        let red = record('a', outcome, GateTree::Clean);
        for gated_for_tip in [false, true] {
            assert_eq!(
                gate_step(&done(&tip, Some(&red), gated_for_tip)),
                GateStep::Retry,
                "{outcome:?}, gated_for_tip={gated_for_tip}"
            );
        }
    }
}

// Rule: a gate that passed on a tree that was not clean is not run again: the same tree gives the same answer.
#[test]
fn a_gate_that_passed_on_a_tree_that_was_not_clean_is_not_run_again() {
    let tip = commit('a');
    let dirty = record('a', GateOutcome::Passed, GateTree::NotClean);

    assert!(matches!(
        gate_step(&done(&tip, Some(&dirty), false)),
        GateStep::SendBack(GateHold::NotPassed { .. })
    ));
}

// Rule: a gate that did not pass at the tip, or passed on a tree that was not clean, sends the lead back with what it
// said and where its output is — a failure after the harness ran the gate once more.
#[test]
fn a_gate_that_did_not_pass_at_the_tip_sends_the_lead_back() {
    let tip = commit('a');
    let cases = [
        (record('a', failed(), GateTree::Clean), "failed (exit 2) at aaaaaaa"),
        (
            record('a', GateOutcome::TimedOut, GateTree::Clean),
            "timed out at aaaaaaa",
        ),
        (
            record('a', GateOutcome::Passed, GateTree::NotClean),
            "passed at aaaaaaa, tree not clean",
        ),
    ];

    for (record, summary) in cases {
        let step = gate_step(&Waiting {
            harness_gate: HarnessGate::Retried,
            ..done(&tip, Some(&record), true)
        });

        assert_eq!(
            step,
            GateStep::SendBack(GateHold::NotPassed {
                summary: summary.to_owned(),
                log: AbsolutePath::from_str("/home/tasks/t/gate-1.log").unwrap(),
            })
        );
    }
}

// Rule: a branch without a commit of its own is not gated and not published: the lead is sent back.
#[test]
fn a_done_lead_without_a_commit_of_its_own_is_sent_back() {
    let tip = commit('a');
    let passed = record('a', GateOutcome::Passed, GateTree::Clean);

    for record in [None, Some(&passed)] {
        let waiting = Waiting {
            own_work: false,
            ..done(&tip, record, false)
        };
        assert_eq!(gate_step(&waiting), GateStep::SendBack(GateHold::NoCommit));
    }
}

// Rule: the harness runs the gate once for a tip; a gate that then is not of the tip ran on another commit.
#[test]
fn a_gate_the_harness_ran_for_the_tip_that_is_of_another_commit_sends_the_lead_back() {
    let tip = commit('b');
    let other = record('a', GateOutcome::Passed, GateTree::Clean);

    assert_eq!(
        gate_step(&done(&tip, Some(&other), true)),
        GateStep::SendBack(GateHold::NotAtTip {
            tip: commit('b'),
            gated: Some(commit('a')),
        })
    );
    assert_eq!(
        gate_step(&done(&tip, None, true)),
        GateStep::SendBack(GateHold::NotAtTip {
            tip: commit('b'),
            gated: None,
        })
    );
}

// Rule: every reason is a sentence the lead can act on.
#[test]
fn the_reasons_say_what_is_wrong_and_where_to_look() {
    let not_passed = GateHold::NotPassed {
        summary: "failed (exit 2) at aaaaaaa".to_owned(),
        log: AbsolutePath::from_str("/home/tasks/t/gate-1.log").unwrap(),
    };
    let not_at_tip = GateHold::NotAtTip {
        tip: commit('b'),
        gated: Some(commit('a')),
    };

    assert_eq!(
        GateHold::NoCommit.to_string(),
        "the branch has no commit of its own, so there is nothing to publish."
    );
    assert_eq!(
        not_passed.to_string(),
        "the gate failed (exit 2) at aaaaaaa; its whole output is in /home/tasks/t/gate-1.log."
    );
    assert!(
        not_at_tip.to_string().contains(&commit('a').to_string()),
        "{not_at_tip}"
    );
    assert!(
        not_at_tip.to_string().contains(&commit('b').to_string()),
        "{not_at_tip}"
    );
}
