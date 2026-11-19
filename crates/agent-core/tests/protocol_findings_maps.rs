//! Tests reproducing the failure to decode maps keyed by finding numbers below tagged enums
//! and verifying round trips across all protocol replies and events.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::history::{HistoryHealth, HistoryState};
use agent_core::ids::{ItemId, RoundId, TeamId};
use agent_core::kinds::Role;
use agent_core::protocol::{
    ControlAction, Event, EventKind, Frame, Outcome, Reply, RequestId, Response, RoundDetail, TeamSummary,
};
use agent_core::state::{
    Findings, GateOutcome, GateRecord, GateResult, GateTree, ItemPhase, Member, Participant, PrNumber, ReviewOutcome,
    RoundEnd, RunEnd, State,
};
use agent_core::status::{ItemStatus, ReviewVerdict, RunOutcome, TaskStatus};
use serde_json::json;

#[path = "support/world.rs"]
mod world;

use world::{at, pr, review, state, task, team_id};

/// Creates a test `Findings` ledger with findings in fix pass:
/// - finding 1: explanation accepted
/// - finding 2: opinion waiting
/// - finding 3: opinion assigned
/// - finding 4: opinion reported not confirmed
/// - finding 5: opinion failed and attempted again
/// - finding 6: fixed
fn fix_pass_findings() -> Findings {
    let raw = json!({
        "entries": [
            {
                "number": 1,
                "finding": {
                    "severity": "MINOR",
                    "rule": "ERR-1",
                    "place": "src/a.rs:1",
                    "text": "explanation accepted",
                    "check": "cargo check"
                },
                "reporter": "inv-codex",
                "pass": 1,
                "status": "open"
            },
            {
                "number": 2,
                "finding": {
                    "severity": "BLOCKER",
                    "rule": "ERR-2",
                    "place": "src/b.rs:2",
                    "text": "opinion requested",
                    "check": "cargo check"
                },
                "reporter": "inv-codex",
                "pass": 1,
                "status": "open"
            },
            {
                "number": 3,
                "finding": {
                    "severity": "BLOCKER",
                    "rule": "ERR-3",
                    "place": "src/c.rs:3",
                    "text": "opinion assigned",
                    "check": "cargo check"
                },
                "reporter": "inv-codex",
                "pass": 1,
                "status": "open"
            },
            {
                "number": 4,
                "finding": {
                    "severity": "BLOCKER",
                    "rule": "ERR-4",
                    "place": "src/d.rs:4",
                    "text": "opinion reported not confirmed",
                    "check": "cargo check"
                },
                "reporter": "inv-codex",
                "pass": 1,
                "status": "not_confirmed"
            },
            {
                "number": 5,
                "finding": {
                    "severity": "BLOCKER",
                    "rule": "ERR-5",
                    "place": "src/e.rs:5",
                    "text": "opinion failed attempt",
                    "check": "cargo check"
                },
                "reporter": "inv-codex",
                "pass": 1,
                "status": "open"
            },
            {
                "number": 6,
                "finding": {
                    "severity": "MAJOR",
                    "rule": "ERR-6",
                    "place": "src/f.rs:6",
                    "text": "fixed finding",
                    "check": "cargo check"
                },
                "reporter": "inv-codex",
                "pass": 1,
                "status": "open"
            }
        ],
        "disputes": {
            "accepted_heads": {
                "1": "1111111111111111111111111111111111111111"
            },
            "changed_findings": [],
            "explanations": {
                "1": {
                    "why": "clarified contract",
                    "status": "accepted",
                    "votes": {
                        "inv-codex": {
                            "status": "accepted",
                            "why": "sufficient contract"
                        }
                    }
                }
            },
            "raised": [],
            "answered_pass": 1,
            "bounces": 0,
            "checked_run": null,
            "correction": null
        },
        "opinions": {
            "2": {
                "request": {
                    "pass": 1,
                    "head": "2222222222222222222222222222222222222222",
                    "reason": "boundary is unreachable"
                },
                "stage": {
                    "stage": "waiting"
                }
            },
            "3": {
                "request": {
                    "pass": 1,
                    "head": "3333333333333333333333333333333333333333",
                    "reason": "lock is safe"
                },
                "stage": {
                    "stage": "assigned",
                    "investigator": "inv-gemini",
                    "task": "task-20261111-193008-k3x9"
                }
            },
            "4": {
                "request": {
                    "pass": 1,
                    "head": "4444444444444444444444444444444444444444",
                    "reason": "false positive"
                },
                "stage": {
                    "stage": "reported",
                    "investigator": "inv-gemini",
                    "task": "task-20261111-193008-k3x9",
                    "report": {
                        "outcome": "not_confirmed",
                        "evidence": "reproduced defect"
                    }
                }
            },
            "5": {
                "request": {
                    "pass": 1,
                    "head": "5555555555555555555555555555555555555555",
                    "reason": "crash is unreachable"
                },
                "stage": {
                    "stage": "waiting"
                }
            }
        },
        "opinion_attempts": {
            "5": [
                {
                    "investigator": "inv-codex",
                    "task": "task-20261111-193007-k3x9",
                    "asked_at": "2026-11-11T19:40:00Z",
                    "failure": "failed"
                }
            ]
        },
        "opinion_notices": [3],
        "fixed": {
            "1": [6]
        }
    });

    serde_json::from_value(raw).unwrap()
}

/// Builds a state containing team 1 with an active round and item 1.2 in fixing phase with opinions.
fn fix_pass_state() -> (State, TeamId, RoundId) {
    let mut world_state = state();
    let team = team_id(1);
    let round_id: RoundId = "round-20261111-193001-q1w2".parse().unwrap();
    let item_id: ItemId = "1.2".parse().unwrap();

    world_state
        .open_round(&team, round_id.clone(), Some(item_id), at(1))
        .unwrap();

    {
        let mut t = world_state.team_mut(&team).unwrap();
        let mut item = t.item_mut(item_id).unwrap();
        item.advance(ItemPhase::InProgress {
            assignee: "codex-1".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
        })
        .unwrap();
        let pull = pr(1);
        item.advance(ItemPhase::InReview {
            pr: pull.clone(),
            reviews: Vec::new(),
        })
        .unwrap();
        item.ask_reviews(vec![
            review("inv-codex", 7, ReviewOutcome::Pending),
            review("inv-gemini", 8, ReviewOutcome::Pending),
        ])
        .unwrap();
        item.submit_review(&task(7), ReviewVerdict::RequestChanges, at(30))
            .unwrap();
        let ItemPhase::InReview { pr: opened, .. } = item.phase().clone() else {
            panic!("in review");
        };
        item.advance(ItemPhase::Fixing {
            assignee: "codex-1".parse().unwrap(),
            branch: opened.branch.clone(),
            pr: opened,
        })
        .unwrap();
        item.start_findings();
        *item.findings_mut().unwrap() = fix_pass_findings();
    }

    (world_state, team, round_id)
}

#[test]
fn round_detail_answer_round_trips_through_response() {
    let (state, team_id, round_id) = fix_pass_state();
    let team = state.team(&team_id).unwrap();
    let round = team.rounds().iter().find(|r| *r.id() == round_id).unwrap();
    let detail = RoundDetail::new(team, round, Vec::new());

    let response = Response::ok(
        RequestId(1),
        Reply::Round {
            round: Box::new(detail.clone()),
        },
    );
    let frame = Frame::Response(response);
    let line = frame.to_line().unwrap();

    let decoded = Frame::from_line(&line).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn team_answer_round_trips_through_response() {
    let (state, team_id, _) = fix_pass_state();
    let team = state.team(&team_id).unwrap();

    let response = Response::ok(
        RequestId(2),
        Reply::Team {
            team: Box::new(team.clone()),
        },
    );
    let frame = Frame::Response(response);
    let line = frame.to_line().unwrap();

    let decoded = Frame::from_line(&line).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn state_v0_11_fixture_with_second_opinions_loads_and_writes_back_unchanged() {
    let fixture_text = include_str!("fixtures/state-v0-11-second-opinions.json");
    let state = State::from_json(fixture_text).unwrap();
    let serialized = format!("{}\n", state.to_json().unwrap());
    assert_eq!(serialized, fixture_text);

    let item = &state.teams()[0].items()[1];
    let ledger = item.findings().unwrap();
    assert_eq!(ledger.entries().len(), 6);
    assert_eq!(ledger.opinions().len(), 4);
    assert_eq!(ledger.opinion_attempts(NonZeroU32::new(5).unwrap()).len(), 1);
    assert_eq!(ledger.fixed_answers(NonZeroU32::new(1).unwrap()).len(), 1);
    assert!(ledger.disputes().is_some());
}

#[test]
fn daemon_answer_fixture_from_main_loads() {
    let fixture_line = include_str!("fixtures/protocol-v3-round-answer-opinions.json");
    let frame = Frame::from_line(fixture_line).unwrap();
    let Frame::Response(response) = &frame else {
        panic!("expected response");
    };
    let Outcome::Ok {
        reply: Reply::Round { round },
    } = &response.result
    else {
        panic!("expected round reply");
    };
    assert_eq!(round.opinions.len(), 4);
    assert_eq!(round.opinion_attempts.len(), 1);
}

#[test]
fn every_reply_variant_round_trips_below_response() {
    let (state, team_id, round_id) = fix_pass_state();
    let team = state.team(&team_id).unwrap();
    let round = team.rounds().iter().find(|r| *r.id() == round_id).unwrap();
    let detail = RoundDetail::new(team, round, Vec::new());
    let summary = TeamSummary::from(team);

    let gate_res = GateResult {
        head: "1111111111111111111111111111111111111111".parse().unwrap(),
        finished_at: at(10),
        outcome: GateOutcome::Passed,
        log: "/tmp/gate.log".parse().unwrap(),
    };

    let replies = vec![
        Reply::Started {
            team: team_id.clone(),
            tasks: vec![task(1), task(2)],
        },
        Reply::Teams {
            teams: vec![summary.clone()],
        },
        Reply::Overview {
            teams: vec![summary.clone()],
            history: HistoryHealth {
                state: HistoryState::Writing,
                lost: 0,
            },
        },
        Reply::Team {
            team: Box::new(team.clone()),
        },
        Reply::Round {
            round: Box::new(detail),
        },
        Reply::Task {
            team: team_id,
            role: Role::Coordinator,
            member: Box::new(Member::new(task(1), Participant::Normal("claude-1".parse().unwrap()))),
        },
        Reply::Applied { tasks: vec![task(1)] },
        Reply::Gate {
            result: gate_res.clone(),
        },
        Reply::GateRecorded {
            record: GateRecord {
                result: gate_res,
                tree: GateTree::Clean,
            },
        },
        Reply::Subscribed { teams: vec![summary] },
    ];

    for (index, reply) in replies.into_iter().enumerate() {
        // Exhaustive match ensures any variant added later fails to compile until covered here.
        match &reply {
            Reply::Started { .. }
            | Reply::Teams { .. }
            | Reply::Overview { .. }
            | Reply::Team { .. }
            | Reply::Round { .. }
            | Reply::Task { .. }
            | Reply::Applied { .. }
            | Reply::Gate { .. }
            | Reply::GateRecorded { .. }
            | Reply::Subscribed { .. } => {}
        }

        let response = Response::ok(RequestId(index as u64 + 1), reply);
        let frame = Frame::Response(response);
        let line = frame.to_line().unwrap();
        let decoded = Frame::from_line(&line).unwrap();
        assert_eq!(decoded, frame);
    }
}

#[test]
fn every_event_kind_variant_round_trips_below_event() {
    let (state, team_id, round_id) = fix_pass_state();
    let team = state.team(&team_id).unwrap();
    let summary = TeamSummary::from(team);

    let events = vec![
        EventKind::TeamStarted { team: team_id.clone() },
        EventKind::TeamUpdated {
            summary: Box::new(summary),
        },
        EventKind::TaskStatus {
            team: team_id.clone(),
            task: task(1),
            status: TaskStatus::Waiting,
        },
        EventKind::RoundOpened {
            team: team_id.clone(),
            round: round_id.clone(),
            item: Some("1.2".parse().unwrap()),
        },
        EventKind::RoundClosed {
            team: team_id.clone(),
            round: round_id.clone(),
            item: Some("1.2".parse().unwrap()),
            how: RoundEnd::Approved,
        },
        EventKind::ItemStatus {
            team: team_id.clone(),
            item: "1.2".parse().unwrap(),
            round: round_id.clone(),
            status: ItemStatus::InProgress,
        },
        EventKind::RunStarted {
            team: team_id.clone(),
            task: task(1),
            run: "run-20261111-193001-r7q2".parse().unwrap(),
            rounds: vec![round_id.clone()],
        },
        EventKind::RunEnded {
            team: team_id.clone(),
            task: task(1),
            run: "run-20261111-193001-r7q2".parse().unwrap(),
            rounds: vec![round_id.clone()],
            end: RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        },
        EventKind::QuestionAsked {
            team: team_id.clone(),
            task: task(1),
            number: NonZeroU32::new(1).unwrap(),
        },
        EventKind::QuestionAnswered {
            team: team_id.clone(),
            task: task(1),
            number: NonZeroU32::new(1).unwrap(),
            via: agent_core::state::AnsweredVia::Command,
        },
        EventKind::PullRequestOpened {
            team: team_id.clone(),
            item: "1.2".parse().unwrap(),
            round: round_id.clone(),
            pr: PrNumber::new(NonZeroU32::new(1).unwrap()),
        },
        EventKind::ReviewSubmitted {
            team: team_id.clone(),
            item: "1.2".parse().unwrap(),
            round: round_id,
            reviewer: "inv-codex".parse().unwrap(),
            verdict: ReviewVerdict::RequestChanges,
        },
        EventKind::GateFinished {
            team: team_id.clone(),
            task: task(1),
            outcome: GateOutcome::Passed,
        },
        EventKind::Controlled {
            team: team_id,
            task: Some(task(1)),
            action: ControlAction::Stop,
            by: agent_core::protocol::Client::Cli,
        },
    ];

    for (index, kind) in events.into_iter().enumerate() {
        // Exhaustive match ensures any variant added later fails to compile until covered here.
        match &kind {
            EventKind::TeamStarted { .. }
            | EventKind::TeamUpdated { .. }
            | EventKind::TaskStatus { .. }
            | EventKind::RoundOpened { .. }
            | EventKind::RoundClosed { .. }
            | EventKind::ItemStatus { .. }
            | EventKind::RunStarted { .. }
            | EventKind::RunEnded { .. }
            | EventKind::QuestionAsked { .. }
            | EventKind::QuestionAnswered { .. }
            | EventKind::PullRequestOpened { .. }
            | EventKind::ReviewSubmitted { .. }
            | EventKind::GateFinished { .. }
            | EventKind::Controlled { .. } => {}
        }

        let event = Event::new(at(index as u32), kind);
        let frame = Frame::Event(event);
        let line = frame.to_line().unwrap();
        let decoded = Frame::from_line(&line).unwrap();
        assert_eq!(decoded, frame);
    }
}

#[test]
fn key_deserialization_accepts_positive_integers_and_refuses_invalid_keys() {
    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct Sample {
        #[serde(deserialize_with = "agent_core::protocol::finding_map::deserialize_map")]
        map: BTreeMap<NonZeroU32, String>,
    }

    // Positive numbers as string or number:
    let valid_json = json!({
        "map": {
            "1": "one",
            "2": "two"
        }
    });
    let parsed: Sample = serde_json::from_value(valid_json).unwrap();
    assert_eq!(parsed.map.len(), 2);
    assert_eq!(parsed.map.get(&NonZeroU32::new(1).unwrap()), Some(&"one".to_string()));
    assert_eq!(parsed.map.get(&NonZeroU32::new(2).unwrap()), Some(&"two".to_string()));

    // Invalid keys must be refused and error message must name the key:
    for invalid in ["0", "-1", "x", ""] {
        let bad_json = json!({
            "map": {
                invalid: "val"
            }
        });
        let err = serde_json::from_value::<Sample>(bad_json).unwrap_err().to_string();
        assert!(
            err.contains(invalid),
            "error should name invalid key {invalid:?}: {err}"
        );
    }
}
