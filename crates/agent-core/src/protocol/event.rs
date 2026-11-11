//! Events: what the daemon tells a subscribed client when the state changes.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::ProtocolVersion;
use super::request::Client;
use crate::ids::{InvestigatorName, ItemId, RunId, TaskId, TeamId};
use crate::state::{AnsweredVia, GateOutcome, PrNumber, RunEnd, Timestamp};
use crate::status::{ItemStatus, ReviewVerdict, TaskStatus};

/// A change of the state, when it happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// The protocol version, always [`PROTOCOL_VERSION`](super::PROTOCOL_VERSION).
    protocol: ProtocolVersion,
    /// When it happened.
    pub at: Timestamp,
    /// What happened.
    pub event: EventKind,
}

/// A control the human used on a task or a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    /// `stop`.
    Stop,
    /// `pause`.
    Pause,
    /// `unpause`.
    Unpause,
    /// `resume`.
    Resume,
    /// `say`.
    Say,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    /// A team was started.
    TeamStarted {
        /// The team.
        team: TeamId,
    },
    /// A task changed its status.
    TaskStatus {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// Its new status.
        status: TaskStatus,
    },
    /// An item changed its status.
    ItemStatus {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// Its new status.
        status: ItemStatus,
    },
    /// A run started.
    RunStarted {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The run.
        run: RunId,
    },
    /// A run ended.
    RunEnded {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The run.
        run: RunId,
        /// How it ended.
        end: RunEnd,
    },
    /// A task put questions to the human.
    QuestionAsked {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The round.
        round: NonZeroU32,
    },
    /// The human answered a round of questions.
    QuestionAnswered {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The round.
        round: NonZeroU32,
        /// Through which channel the answers came.
        via: AnsweredVia,
    },
    /// A pull request was opened.
    PullRequestOpened {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// The pull request.
        pr: PrNumber,
    },
    /// A reviewer's review came in.
    ReviewSubmitted {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// The reviewer.
        reviewer: InvestigatorName,
        /// The verdict.
        verdict: ReviewVerdict,
    },
    /// The gate ended.
    GateFinished {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// What it said.
        outcome: GateOutcome,
    },
    /// The human used a control.
    Controlled {
        /// The team.
        team: TeamId,
        /// The task, or `None` when the control was for the whole team.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<TaskId>,
        /// Which control.
        action: ControlAction,
        /// Who used it.
        by: Client,
    },
}

impl Event {
    /// An event that happened at `at`.
    #[must_use]
    pub const fn new(at: Timestamp, event: EventKind) -> Self {
        Self {
            protocol: ProtocolVersion,
            at,
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::fixtures::{assert_refused, at, task, team_id};

    /// The sample task and team of these tests, as the text of their IDs.
    const TASK: &str = "task-20261111-153002-k3x9";
    const TEAM: &str = "team-20261111-153001-t5m8";
    const RUN: &str = "run-20261111-153003-r7q2";

    /// The run of these tests.
    fn run() -> RunId {
        RUN.parse().unwrap()
    }

    /// Asserts that each event is written as its JSON and that the JSON reads back as the event.
    fn assert_json_round_trips(cases: Vec<(EventKind, serde_json::Value)>) {
        for (kind, expected) in cases {
            let value = serde_json::to_value(&kind).unwrap();

            assert_eq!(value, expected);
            assert_eq!(serde_json::from_value::<EventKind>(value).unwrap(), kind);
        }
    }

    #[test]
    fn the_events_of_teams_tasks_items_and_runs_have_their_json_and_read_back_equal() {
        let item: ItemId = "1.2".parse().unwrap();
        let cases: Vec<(EventKind, serde_json::Value)> = vec![
            (
                EventKind::TeamStarted { team: team_id(1) },
                json!({"kind": "team_started", "team": TEAM}),
            ),
            (
                EventKind::TaskStatus {
                    team: team_id(1),
                    task: task(2),
                    status: TaskStatus::Questions,
                },
                json!({"kind": "task_status", "team": TEAM, "task": TASK, "status": "questions"}),
            ),
            (
                EventKind::ItemStatus {
                    team: team_id(1),
                    item,
                    status: ItemStatus::InReview,
                },
                json!({"kind": "item_status", "team": TEAM, "item": "1.2", "status": "in_review"}),
            ),
            (
                EventKind::RunStarted {
                    team: team_id(1),
                    task: task(2),
                    run: run(),
                },
                json!({"kind": "run_started", "team": TEAM, "task": TASK, "run": RUN}),
            ),
            (
                EventKind::RunEnded {
                    team: team_id(1),
                    task: task(2),
                    run: run(),
                    end: RunEnd::Hung,
                },
                json!({"kind": "run_ended", "team": TEAM, "task": TASK, "run": RUN, "end": {"kind": "hung"}}),
            ),
        ];
        assert_json_round_trips(cases);
    }

    #[test]
    fn the_events_of_questions_pull_requests_reviews_gates_and_controls_have_their_json_and_read_back_equal() {
        let item: ItemId = "1.2".parse().unwrap();
        let cases: Vec<(EventKind, serde_json::Value)> = vec![
            (
                EventKind::QuestionAsked {
                    team: team_id(1),
                    task: task(2),
                    round: NonZeroU32::MIN,
                },
                json!({"kind": "question_asked", "team": TEAM, "task": TASK, "round": 1}),
            ),
            (
                EventKind::QuestionAnswered {
                    team: team_id(1),
                    task: task(2),
                    round: NonZeroU32::MIN,
                    via: AnsweredVia::Dashboard,
                },
                json!({"kind": "question_answered", "team": TEAM, "task": TASK, "round": 1, "via": "dashboard"}),
            ),
            (
                EventKind::PullRequestOpened {
                    team: team_id(1),
                    item,
                    pr: PrNumber::new(NonZeroU32::new(82).unwrap()),
                },
                json!({"kind": "pull_request_opened", "team": TEAM, "item": "1.2", "pr": 82}),
            ),
            (
                EventKind::ReviewSubmitted {
                    team: team_id(1),
                    item,
                    reviewer: "inv-codex".parse().unwrap(),
                    verdict: ReviewVerdict::RequestChanges,
                },
                json!({"kind": "review_submitted", "team": TEAM, "item": "1.2", "reviewer": "inv-codex", "verdict": "REQUEST_CHANGES"}),
            ),
            (
                EventKind::GateFinished {
                    team: team_id(1),
                    task: task(2),
                    outcome: GateOutcome::Failed { exit_code: 2 },
                },
                json!({"kind": "gate_finished", "team": TEAM, "task": TASK, "outcome": {"kind": "failed", "exit_code": 2}}),
            ),
            (
                EventKind::Controlled {
                    team: team_id(1),
                    task: Some(task(2)),
                    action: ControlAction::Pause,
                    by: Client::Cli,
                },
                json!({"kind": "controlled", "team": TEAM, "task": TASK, "action": "pause", "by": {"kind": "cli"}}),
            ),
            (
                EventKind::Controlled {
                    team: team_id(1),
                    task: None,
                    action: ControlAction::Say,
                    by: Client::Dashboard {
                        login: "you@example.com".parse().unwrap(),
                    },
                },
                json!({"kind": "controlled", "team": TEAM, "action": "say", "by": {"kind": "dashboard", "login": "you@example.com"}}),
            ),
        ];
        assert_json_round_trips(cases);
    }

    #[test]
    fn an_event_carries_its_time_and_the_version() {
        let event = Event::new(at(30), EventKind::TeamStarted { team: team_id(1) });

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(
            value,
            json!({"protocol": 1, "at": "2026-11-11T19:15:30Z", "event": {"kind": "team_started", "team": TEAM}})
        );
        assert_eq!(serde_json::from_value::<Event>(value).unwrap(), event);
        assert_eq!(
            (event.at, &event.event),
            (at(30), &EventKind::TeamStarted { team: team_id(1) })
        );
    }

    #[test]
    fn the_controls_have_their_texts() {
        for (action, text) in [
            (ControlAction::Stop, "stop"),
            (ControlAction::Pause, "pause"),
            (ControlAction::Unpause, "unpause"),
            (ControlAction::Resume, "resume"),
            (ControlAction::Say, "say"),
        ] {
            assert_eq!(serde_json::to_value(action).unwrap(), json!(text));
            assert_eq!(serde_json::from_value::<ControlAction>(json!(text)).unwrap(), action);
        }
        assert_refused::<ControlAction>(&json!("kill"));
    }

    #[test]
    fn an_event_with_a_bad_part_is_refused() {
        for value in [
            json!({"kind": "team_started"}),
            json!({"kind": "team_started", "team": TASK}),
            json!({"kind": "task_status", "team": TEAM, "task": TASK, "status": "sleeping"}),
            json!({"kind": "question_asked", "team": TEAM, "task": TASK, "round": 0}),
            json!({"kind": "controlled", "team": TEAM, "action": "pause"}),
            json!({"kind": "teleported", "team": TEAM}),
            json!({"kind": "team_started", "team": TEAM, "extra": 1}),
        ] {
            assert_refused::<EventKind>(&value);
        }
        assert_refused::<Event>(
            &json!({"protocol": 2, "at": "2026-11-11T19:15:30Z", "event": {"kind": "team_started", "team": TEAM}}),
        );
        assert_refused::<Event>(
            &json!({"protocol": 1, "at": "yesterday", "event": {"kind": "team_started", "team": TEAM}}),
        );
    }
}
