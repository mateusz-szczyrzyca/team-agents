//! The record of the end of a worker's request: its scope and its fields for each outcome, with and without cost.

use agent_core::log::LogEvent;
use agent_core::state::Timestamp;
use agent_runtime::agentbus::RequestId;
use agent_runtime::worker_request::Settlement;
use serde_json::{Value, json};
use std::num::NonZeroU64;

use super::{EndOutcome, RequestEnd};

/// Noon of the day the tests are set in.
fn noon() -> Timestamp {
    "2026-11-25T12:00:00Z".parse().unwrap()
}

/// The end of the request H3 of `outcome`, as it is after a worker's 90 seconds from 11:58:30.
fn end(outcome: EndOutcome<'_>) -> RequestEnd<'_> {
    RequestEnd {
        request: RequestId::new(NonZeroU64::new(3).unwrap()),
        team: "team-20261125-072656-6axz".parse().unwrap(),
        rounds: vec!["round-20261125-072700-aaaa".parse().unwrap()],
        requester: "claude-1".parse().unwrap(),
        requester_task: "task-20261125-072656-gnck".parse().unwrap(),
        step: Some("2.1".parse().unwrap()),
        worker: "glm-1".parse().unwrap(),
        worker_task: "task-20261125-072700-wxyz".parse().unwrap(),
        try_number: 1,
        outcome,
        started: Some("2026-11-25T11:58:30Z".parse().unwrap()),
        seconds: 90,
        cost_usd: Some(0.0123),
    }
}

/// The fields of the record of `end`, as JSON.
fn fields_of(end: &RequestEnd<'_>) -> Value {
    serde_json::to_value(end.record(noon()).fields).unwrap()
}

#[test]
fn the_record_belongs_to_the_requesters_team_round_and_task() {
    let brought = Settlement::Brought {
        commit: "abc".to_owned(),
    };
    let record = end(EndOutcome::Settled(&brought)).record(noon());

    assert_eq!(record.event, LogEvent::WorkerRequestEnded);
    assert_eq!(record.at, noon());
    assert_eq!(
        record.team_id().map(ToString::to_string).as_deref(),
        Some("team-20261125-072656-6axz")
    );
    assert_eq!(
        record.rounds().iter().map(ToString::to_string).collect::<Vec<_>>(),
        vec!["round-20261125-072700-aaaa".to_owned()]
    );
    assert_eq!(
        record.task_id().map(ToString::to_string).as_deref(),
        Some("task-20261125-072656-gnck")
    );
}

#[test]
fn a_record_has_every_field_with_the_cost_told() {
    let brought = Settlement::Brought {
        commit: "abc".to_owned(),
    };

    assert_eq!(
        fields_of(&end(EndOutcome::Settled(&brought))),
        json!({
            "request": "H3",
            "task": "task-20261125-072656-gnck",
            "requester": "claude-1",
            "step": "2.1",
            "worker": "glm-1",
            "worker_task": "task-20261125-072700-wxyz",
            "try": 1,
            "outcome": "brought",
            "started": "2026-11-25T11:58:30Z",
            "seconds": 90,
            "cost_usd": "0.0123",
        })
    );
}

#[test]
fn a_record_leaves_the_step_the_start_and_the_cost_empty_when_they_are_not_known() {
    let mut unknown = end(EndOutcome::Requeued);
    unknown.step = None;
    unknown.started = None;
    unknown.cost_usd = None;
    unknown.rounds = Vec::new();

    let fields = fields_of(&unknown);

    assert_eq!(fields["step"], json!(""));
    assert_eq!(fields["started"], json!(""));
    assert_eq!(fields["cost_usd"], json!(""));
    assert_eq!(unknown.record(noon()).rounds().len(), 0);
}

#[test]
fn every_outcome_has_its_word() {
    let reasons = vec!["wrong".to_owned()];
    let report = Settlement::Report;
    let brought = Settlement::Brought {
        commit: "abc".to_owned(),
    };
    let conflict = Settlement::Conflict {
        files: vec!["a.txt".to_owned()],
    };
    let again = Settlement::Again {
        reasons: reasons.clone(),
    };
    let returned = Settlement::Returned { reasons };
    let cases = [
        (EndOutcome::Settled(&report), "report"),
        (EndOutcome::Settled(&brought), "brought"),
        (EndOutcome::Settled(&conflict), "conflict"),
        (EndOutcome::Settled(&again), "again"),
        (EndOutcome::Settled(&returned), "returned"),
        (EndOutcome::Requeued, "worker_request_requeued"),
        (EndOutcome::ReturnedUnfinished, "worker_request_returned"),
    ];

    for (outcome, word) in cases {
        assert_eq!(outcome.word(), word);
        assert_eq!(fields_of(&end(outcome))["outcome"], json!(word));
    }
}
