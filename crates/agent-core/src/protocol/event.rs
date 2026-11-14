//! Events: what the daemon tells a subscribed client when the state changes.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::ProtocolVersion;
use super::reply::TeamSummary;
use super::request::Client;
use crate::ids::{InvestigatorName, ItemId, RoundId, RunId, TaskId, TeamId};
use crate::state::{AnsweredVia, GateOutcome, PrNumber, RoundEnd, RunEnd, Timestamp};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
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
    /// A team's summary changed: the whole new summary, which replaces the one a client has.
    ///
    /// The daemon sends it after every change that alters a summary — a team started, a task's status, an item's
    /// status, a question asked or answered — right after the event that says what happened. A client that keeps
    /// summaries needs no other event for them: see [`EventKind::apply_to`].
    TeamUpdated {
        /// The team's summary as it is now.
        summary: Box<TeamSummary>,
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
    /// A round was opened: its item was given to a member, or the team started on a list without items.
    RoundOpened {
        /// The team.
        team: TeamId,
        /// The round.
        round: RoundId,
        /// The item the round is for; none for the whole work of a team whose list has no items.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
    },
    /// A round was closed.
    RoundClosed {
        /// The team.
        team: TeamId,
        /// The round.
        round: RoundId,
        /// The item the round is for; none for the whole work of a team whose list has no items.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
        /// How it ended.
        how: RoundEnd,
    },
    /// An item changed its status.
    ItemStatus {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// The item's round.
        round: RoundId,
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
        /// The rounds the run works for; none for a run that works for no item.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rounds: Vec<RoundId>,
    },
    /// A run ended.
    RunEnded {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The run.
        run: RunId,
        /// The rounds the run worked for; none for a run that worked for no item.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rounds: Vec<RoundId>,
        /// How it ended.
        end: RunEnd,
    },
    /// A task put questions to the human.
    QuestionAsked {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The number of the questions.
        number: NonZeroU32,
    },
    /// The human answered questions.
    QuestionAnswered {
        /// The team.
        team: TeamId,
        /// The task.
        task: TaskId,
        /// The number of the questions.
        number: NonZeroU32,
        /// Through which channel the answers came.
        via: AnsweredVia,
    },
    /// A pull request was opened.
    PullRequestOpened {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// The item's round.
        round: RoundId,
        /// The pull request.
        pr: PrNumber,
    },
    /// A reviewer's review came in.
    ReviewSubmitted {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// The item's round.
        round: RoundId,
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

impl EventKind {
    /// Brings `teams`, the summaries of a subscription (the `teams` of [`Reply::Subscribed`](super::Reply)), up to
    /// date with this event.
    ///
    /// Only [`EventKind::TeamUpdated`] changes them: it replaces the summary of its team, or adds it at the end when
    /// the client has none (a team started after the subscription; teams are listed oldest first). The other events
    /// say what happened and carry too little to change a summary on their own, so they leave `teams` as it is.
    pub fn apply_to(&self, teams: &mut Vec<TeamSummary>) {
        let Self::TeamUpdated { summary } = self else {
            return;
        };
        match teams.iter_mut().find(|known| known.id == summary.id) {
            Some(known) => known.clone_from(summary),
            None => teams.push(summary.as_ref().clone()),
        }
    }
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
    use crate::state::fixtures::{assert_refused, at, round_id, task, team_id};

    /// The sample task and team of these tests, as the text of their IDs.
    const TASK: &str = "task-20261111-153002-k3x9";
    const TEAM: &str = "team-20261111-153001-t5m8";
    const RUN: &str = "run-20261111-153003-r7q2";
    const ROUND: &str = "round-20261111-153001-q1w2";

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
                    round: round_id(1),
                    status: ItemStatus::InReview,
                },
                json!({"kind": "item_status", "team": TEAM, "item": "1.2", "round": ROUND, "status": "in_review"}),
            ),
            (
                EventKind::RunStarted {
                    team: team_id(1),
                    task: task(2),
                    run: run(),
                    rounds: vec![round_id(1)],
                },
                json!({"kind": "run_started", "team": TEAM, "task": TASK, "run": RUN, "rounds": [ROUND]}),
            ),
            (
                EventKind::RunEnded {
                    team: team_id(1),
                    task: task(2),
                    run: run(),
                    rounds: Vec::new(),
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
                    number: NonZeroU32::MIN,
                },
                json!({"kind": "question_asked", "team": TEAM, "task": TASK, "number": 1}),
            ),
            (
                EventKind::QuestionAnswered {
                    team: team_id(1),
                    task: task(2),
                    number: NonZeroU32::MIN,
                    via: AnsweredVia::Dashboard,
                },
                json!({"kind": "question_answered", "team": TEAM, "task": TASK, "number": 1, "via": "dashboard"}),
            ),
            (
                EventKind::PullRequestOpened {
                    team: team_id(1),
                    item,
                    round: round_id(1),
                    pr: PrNumber::new(NonZeroU32::new(82).unwrap()),
                },
                json!({"kind": "pull_request_opened", "team": TEAM, "item": "1.2", "round": ROUND, "pr": 82}),
            ),
            (
                EventKind::ReviewSubmitted {
                    team: team_id(1),
                    item,
                    round: round_id(1),
                    reviewer: "inv-codex".parse().unwrap(),
                    verdict: ReviewVerdict::RequestChanges,
                },
                json!({"kind": "review_submitted", "team": TEAM, "item": "1.2", "round": ROUND, "reviewer": "inv-codex", "verdict": "REQUEST_CHANGES"}),
            ),
            (
                EventKind::GateFinished {
                    team: team_id(1),
                    task: task(2),
                    outcome: GateOutcome::Failed {
                        exit_code: std::num::NonZeroI32::new(2).unwrap(),
                    },
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
    fn the_events_of_a_round_opened_or_closed_have_their_json_and_read_back_equal() {
        let item: ItemId = "1.2".parse().unwrap();
        let cases: Vec<(EventKind, serde_json::Value)> = vec![
            (
                EventKind::RoundOpened {
                    team: team_id(1),
                    round: round_id(1),
                    item: Some(item),
                },
                json!({"kind": "round_opened", "team": TEAM, "round": ROUND, "item": "1.2"}),
            ),
            (
                EventKind::RoundOpened {
                    team: team_id(1),
                    round: round_id(1),
                    item: None,
                },
                json!({"kind": "round_opened", "team": TEAM, "round": ROUND}),
            ),
            (
                EventKind::RoundClosed {
                    team: team_id(1),
                    round: round_id(1),
                    item: Some(item),
                    how: RoundEnd::Merged,
                },
                json!({"kind": "round_closed", "team": TEAM, "round": ROUND, "item": "1.2", "how": "merged"}),
            ),
            (
                EventKind::RoundClosed {
                    team: team_id(1),
                    round: round_id(1),
                    item: None,
                    how: RoundEnd::Abandoned,
                },
                json!({"kind": "round_closed", "team": TEAM, "round": ROUND, "how": "abandoned"}),
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
            json!({"protocol": 2, "at": "2026-11-11T19:15:30Z", "event": {"kind": "team_started", "team": TEAM}})
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
    fn a_control_names_itself_as_the_human_types_it() {
        for (action, text) in [
            (ControlAction::Stop, "stop"),
            (ControlAction::Pause, "pause"),
            (ControlAction::Unpause, "unpause"),
            (ControlAction::Resume, "resume"),
            (ControlAction::Say, "say"),
        ] {
            assert_eq!(<&'static str>::from(action), text);
        }
    }

    #[test]
    fn an_event_with_a_bad_part_is_refused() {
        for value in [
            json!({"kind": "team_started"}),
            json!({"kind": "team_started", "team": TASK}),
            json!({"kind": "task_status", "team": TEAM, "task": TASK, "status": "sleeping"}),
            json!({"kind": "question_asked", "team": TEAM, "task": TASK, "number": 0}),
            json!({"kind": "controlled", "team": TEAM, "action": "pause"}),
            json!({"kind": "teleported", "team": TEAM}),
            json!({"kind": "team_started", "team": TEAM, "extra": 1}),
            json!({"kind": "round_opened", "team": TEAM, "round": TEAM}),
            json!({"kind": "round_closed", "team": TEAM, "round": ROUND}),
            json!({"kind": "round_closed", "team": TEAM, "round": ROUND, "how": "lost"}),
            json!({"kind": "item_status", "team": TEAM, "item": "1.2", "status": "in_review"}),
        ] {
            assert_refused::<EventKind>(&value);
        }
        assert_refused::<Event>(
            &json!({"protocol": 1, "at": "2026-11-11T19:15:30Z", "event": {"kind": "team_started", "team": TEAM}}),
        );
        assert_refused::<Event>(
            &json!({"protocol": 2, "at": "yesterday", "event": {"kind": "team_started", "team": TEAM}}),
        );
    }
}
