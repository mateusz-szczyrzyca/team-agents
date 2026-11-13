//! Responses: what the daemon answers to a request.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::ProtocolVersion;
use super::request::RequestId;
use crate::config::{AbsolutePath, RepoName};
use crate::ids::{BranchName, ItemId, TaskId, TeamId};
use crate::kinds::Role;
use crate::state::{
    GateRecord, GateResult, ItemPhase, ItemProgress, Member, MergeMode, Participant, PullRequest, Question, Review,
    Round, Run, Team, Timestamp,
};
use crate::status::{ItemStatus, TaskStatus};

/// The answer to one [`Request`](super::Request): the same [`RequestId`] and a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// The protocol version, always [`PROTOCOL_VERSION`](super::PROTOCOL_VERSION).
    protocol: ProtocolVersion,
    /// The id of the request this answers.
    pub id: RequestId,
    /// What came of it.
    pub result: Outcome,
}

/// What came of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    /// The request was carried out.
    Ok {
        /// What it gives back.
        reply: Reply,
    },
    /// The request was not carried out.
    Error {
        /// Why not.
        error: Failure,
    },
}

/// What a request that was carried out gives back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    /// `run`: the team that was started and the tasks of its crew.
    Started {
        /// The team.
        team: TeamId,
        /// Its tasks: the coordinator's (or the solo agent's) first, then the members', then the workers'.
        tasks: Vec<TaskId>,
    },
    /// `status`: the teams in short.
    Teams {
        /// One summary for each team asked for.
        teams: Vec<TeamSummary>,
    },
    /// `show` of a team: the whole record.
    Team {
        /// The team.
        team: Box<Team>,
    },
    /// `show` of a round: its item, its state, the tasks that worked for it, its pull request and its review passes.
    Round {
        /// The round, in full.
        round: Box<RoundDetail>,
    },
    /// `show` of a task: its record.
    Task {
        /// The team the task is in.
        team: TeamId,
        /// The member's role in it.
        role: Role,
        /// The member and its task.
        member: Box<Member>,
    },
    /// `answer`, `stop`, `pause`, `unpause`, `resume` and `say`: the tasks the command reached.
    Applied {
        /// The tasks that changed or received it.
        tasks: Vec<TaskId>,
    },
    /// `gate`: what the gate said, without the state of the tree. The daemon answers with [`Reply::GateRecorded`]
    /// instead; this reply stays so that a client that knows it is still read.
    Gate {
        /// The result.
        result: GateResult,
    },
    /// `gate`: what the daemon recorded about the run, with the state of the tree it ran on.
    GateRecorded {
        /// The record.
        record: GateRecord,
    },
    /// `subscribe`: the teams subscribed to as they are now, in short; the events follow on this connection and
    /// change them from this state on, so a dashboard starts without a gap or a race. A client keeps the summaries
    /// up to date with [`EventKind::apply_to`](super::EventKind::apply_to): each `team_updated` event carries the
    /// whole new summary of its team.
    Subscribed {
        /// One summary for each team subscribed to.
        teams: Vec<TeamSummary>,
    },
}

/// Why a request was not carried out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    /// What kind of failure it is, for the client to act on.
    pub kind: FailureKind,
    /// What went wrong and, where possible, what to do instead; for the human, with no internals.
    pub message: String,
}

/// The kinds of failure a client can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The request is well-formed but asks for something that makes no sense: an unknown agent, a worker as coordinator,
    /// answers that do not fit the questions.
    Invalid,
    /// The team or task asked for is not there.
    NotFound,
    /// The task's state does not allow it now: stopping a task that is done, answering questions not asked.
    Conflict,
    /// The harness refuses to start: too many investigators in the team for two outside reviewers.
    Refused,
    /// The daemon failed; the details are in its log.
    Internal,
}

/// A team in short: for `status` and the dashboard's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamSummary {
    /// The team's ID.
    pub id: TeamId,
    /// The repository.
    pub repo: RepoName,
    /// When the team was started.
    pub created_at: Timestamp,
    /// What happens to an approved pull request.
    pub merge: MergeMode,
    /// How many items are in each status.
    pub items: ItemCounts,
    /// The rounds of the team, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rounds: Vec<Round>,
    /// The crew and the reviewers.
    pub members: Vec<MemberSummary>,
}

/// A round in full: for `show` of a round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundDetail {
    /// The team the round is of.
    pub team: TeamId,
    /// The round: its ID, its item, when it was opened and whether it is closed.
    pub round: Round,
    /// The round's item; none for the whole work of a team whose list has no items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<RoundItem>,
    /// The tasks that worked for the round, in the order of the team (the crew, then the reviewers), each with the runs
    /// that named the round.
    pub tasks: Vec<RoundTask>,
    /// The pull request of the round's item, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<PullRequest>,
    /// The review passes of the pull request, oldest first: each fix pass, then the reviews of the pass that is going.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_passes: Vec<ReviewPass>,
}

/// The item of a round in short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundItem {
    /// The item's ID.
    pub id: ItemId,
    /// Its title in the task list.
    pub title: String,
    /// Where it is now.
    pub status: ItemStatus,
}

/// A task that worked for a round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundTask {
    /// The task.
    pub task: TaskId,
    /// Its role in the team.
    pub role: Role,
    /// Who works in it.
    pub agent: Participant,
    /// The git worktree it works in.
    pub worktree: AbsolutePath,
    /// The branch it works on.
    pub branch: BranchName,
    /// The runs that named the round, oldest first.
    pub runs: Vec<Run>,
}

/// The reviews asked for one head of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPass {
    /// The reviews, in the order asked.
    pub reviews: Vec<Review>,
}

/// How many items of a task list are in each status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemCounts {
    /// Items not started.
    pub pending: u32,
    /// Items a member works on, fix passes included.
    pub in_progress: u32,
    /// Items whose pull request waits for its reviews.
    pub in_review: u32,
    /// Items both reviews approve.
    pub approved: u32,
    /// Items merged or listed as done.
    pub done: u32,
}

impl ItemCounts {
    /// The number of items in `status`.
    #[must_use]
    pub const fn get(&self, status: ItemStatus) -> u32 {
        match status {
            ItemStatus::Pending => self.pending,
            ItemStatus::InProgress => self.in_progress,
            ItemStatus::InReview => self.in_review,
            ItemStatus::Approved => self.approved,
            ItemStatus::Done => self.done,
        }
    }
}

/// A member in short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberSummary {
    /// The member's task.
    pub task: TaskId,
    /// Who works in it.
    pub agent: Participant,
    /// Its role.
    pub role: Role,
    /// Where the task is in its life.
    pub status: TaskStatus,
    /// The number of the questions that wait for the human, if there are any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_question: Option<NonZeroU32>,
}

impl ItemCounts {
    /// Counts one more item in `status`.
    const fn count(&mut self, status: ItemStatus) {
        let slot = match status {
            ItemStatus::Pending => &mut self.pending,
            ItemStatus::InProgress => &mut self.in_progress,
            ItemStatus::InReview => &mut self.in_review,
            ItemStatus::Approved => &mut self.approved,
            ItemStatus::Done => &mut self.done,
        };
        *slot = slot.saturating_add(1);
    }
}

impl MemberSummary {
    /// The summary of `member`, who has `role` in the team.
    fn of(role: Role, member: &Member) -> Self {
        Self {
            task: member.task().clone(),
            agent: member.agent().clone(),
            role,
            status: member.status(),
            open_question: member.open_question().map(Question::number),
        }
    }
}

impl RoundDetail {
    /// The round `round` of `team`, with the `tasks` that worked for it: its pull request and its review passes are
    /// those of its item, which the state keeps (the reviews of the pass that was approved are not kept once the item
    /// is done).
    #[must_use]
    pub fn new(team: &Team, round: &Round, tasks: Vec<RoundTask>) -> Self {
        let progress = round.item().and_then(|item| team.item(item));
        let pr = progress.and_then(|progress| match progress.phase() {
            ItemPhase::InReview { pr, .. }
            | ItemPhase::Fixing { pr, .. }
            | ItemPhase::Approved { pr, .. }
            | ItemPhase::Done {
                completion: crate::state::Completion::Merged { pr, .. },
            } => Some(pr.clone()),
            ItemPhase::Pending
            | ItemPhase::InProgress { .. }
            | ItemPhase::Done {
                completion: crate::state::Completion::Listed,
            } => None,
        });
        let mut review_passes: Vec<ReviewPass> = progress
            .map(|progress| {
                progress
                    .fix_passes()
                    .iter()
                    .map(|pass| ReviewPass {
                        reviews: pass.requested_by.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(ItemPhase::InReview { reviews, .. } | ItemPhase::Approved { reviews, .. }) =
            progress.map(ItemProgress::phase)
        {
            review_passes.push(ReviewPass {
                reviews: reviews.clone(),
            });
        }
        let item = progress.and_then(|progress| {
            team.task_list().list().item(progress.id()).map(|listed| RoundItem {
                id: progress.id(),
                title: listed.title.clone(),
                status: progress.status(),
            })
        });
        Self {
            team: team.id().clone(),
            round: round.clone(),
            item,
            tasks,
            pr,
            review_passes,
        }
    }
}

impl From<&Team> for TeamSummary {
    fn from(team: &Team) -> Self {
        let mut items = ItemCounts::default();
        for item in team.items() {
            items.count(item.status());
        }
        let crew = team.crew().roster().into_iter();
        let reviewers = team.reviewers().iter().map(|member| (Role::Reviewer, member));
        Self {
            id: team.id().clone(),
            repo: team.repo().clone(),
            created_at: team.created_at(),
            merge: team.merge(),
            items,
            rounds: team.rounds().to_vec(),
            members: crew
                .chain(reviewers)
                .map(|(role, member)| MemberSummary::of(role, member))
                .collect(),
        }
    }
}

impl Response {
    /// The answer `reply` to request `id`.
    #[must_use]
    pub const fn ok(id: RequestId, reply: Reply) -> Self {
        Self {
            protocol: ProtocolVersion,
            id,
            result: Outcome::Ok { reply },
        }
    }

    /// The refusal of request `id`.
    #[must_use]
    pub const fn error(id: RequestId, error: Failure) -> Self {
        Self {
            protocol: ProtocolVersion,
            id,
            result: Outcome::Error { error },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::fixtures::{assert_refused, at, inv, member, nz, path, task, team, team_id};
    use crate::state::{AnsweredVia, ItemPhase, Question, State};

    /// The sample task and team of these tests, as the text of their IDs.
    const TASK: &str = "task-20261111-153002-k3x9";
    const TEAM: &str = "team-20261111-153001-t5m8";

    /// A state with the sample team (item 1.2 in progress, `codex-1` asking questions number 1) and the reviewer `inv-codex`.
    fn state() -> State {
        let mut state = State::new();
        let mut team = team(1);
        team.open_round(
            "round-20261111-153001-k3x9".parse().unwrap(),
            Some("1.2".parse().unwrap()),
            at(1),
        )
        .unwrap();
        team.item_mut("1.2".parse().unwrap())
            .unwrap()
            .advance(ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();
        let (_, mut codex) = team.crew_mut().find_mut(&task(2)).unwrap();
        codex
            .ask_question(Question::new(nz(1), path("/home/u/tasks/q-1.md"), at(30)))
            .unwrap();
        codex.transition(TaskStatus::Running).unwrap();
        codex.transition(TaskStatus::Questions).unwrap();
        state.insert_team(team).unwrap();
        state.add_reviewer(&team_id(1), member(7, inv("inv-codex"))).unwrap();
        state
    }

    // ---- the summary of a team ----------------------------------------------------------------------------------

    #[test]
    fn a_team_summary_counts_its_items_and_lists_its_crew_and_reviewers() {
        let state = state();

        let summary = TeamSummary::from(state.team(&team_id(1)).unwrap());

        assert_eq!(summary.id, team_id(1));
        assert_eq!(summary.repo.as_str(), "team-agents");
        assert_eq!(summary.created_at, at(0));
        assert_eq!(summary.merge, MergeMode::Autopilot);
        assert_eq!(
            summary.items,
            ItemCounts {
                pending: 1,
                in_progress: 1,
                in_review: 0,
                approved: 0,
                done: 1
            }
        );
        let members: Vec<(String, Role, TaskStatus, Option<u32>)> = summary
            .members
            .iter()
            .map(|m| {
                (
                    m.agent.name().to_owned(),
                    m.role,
                    m.status,
                    m.open_question.map(NonZeroU32::get),
                )
            })
            .collect();
        assert_eq!(
            members,
            [
                (String::from("inv-opus"), Role::Coordinator, TaskStatus::Queued, None),
                (String::from("codex-1"), Role::Member, TaskStatus::Questions, Some(1)),
                (String::from("worker-1"), Role::Worker, TaskStatus::Queued, None),
                (String::from("inv-codex"), Role::Reviewer, TaskStatus::Queued, None),
            ]
        );
        assert_eq!(summary.members[1].task, task(2));
    }

    #[test]
    fn the_question_of_a_member_is_open_only_until_it_is_answered() {
        let mut state = state();
        state
            .find_task_mut(&task(2))
            .unwrap()
            .1
            .answer_question(nz(1), at(40), AnsweredVia::Command)
            .unwrap();

        let summary = TeamSummary::from(state.team(&team_id(1)).unwrap());

        assert_eq!(summary.members[1].open_question, None);
    }

    #[test]
    fn item_counts_are_looked_up_by_status() {
        let counts = ItemCounts {
            pending: 1,
            in_progress: 2,
            in_review: 3,
            approved: 4,
            done: 5,
        };

        let by_status: Vec<u32> = ItemStatus::ALL.iter().map(|&s| counts.get(s)).collect();

        assert_eq!(by_status, [1, 2, 3, 4, 5]);
        assert_eq!(ItemCounts::default().get(ItemStatus::Done), 0);
    }

    #[test]
    fn a_subscription_starts_from_the_summaries_of_its_teams() {
        let state = state();
        let summary = TeamSummary::from(state.team(&team_id(1)).unwrap());
        let reply = Reply::Subscribed {
            teams: vec![summary.clone()],
        };

        let value = serde_json::to_value(&reply).unwrap();

        assert_eq!(
            value,
            json!({"kind": "subscribed", "teams": [serde_json::to_value(&summary).unwrap()]})
        );
        assert_eq!(serde_json::from_value::<Reply>(value).unwrap(), reply);
    }

    #[test]
    fn a_team_summary_is_written_with_counts_and_members_and_reads_back() {
        let state = state();
        let summary = TeamSummary::from(state.team(&team_id(1)).unwrap());

        let value = serde_json::to_value(&summary).unwrap();

        assert_eq!(
            value["items"],
            json!({"pending": 1, "in_progress": 1, "in_review": 0, "approved": 0, "done": 1})
        );
        assert_eq!(value["members"][1]["open_question"], json!(1));
        assert!(value["members"][0].get("open_question").is_none());
        assert_eq!(
            value["members"][1]["agent"],
            json!({"tier": "normal", "name": "codex-1"})
        );
        assert_eq!(value["members"][1]["role"], json!("member"));
        assert_eq!(serde_json::from_value::<TeamSummary>(value).unwrap(), summary);
    }

    // ---- the replies --------------------------------------------------------------------------------------------

    #[test]
    fn every_reply_has_its_json_and_reads_back_equal() {
        let state = state();
        let found = state.find_task(&task(2)).unwrap();
        let gate = GateResult {
            head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
            finished_at: at(60),
            outcome: crate::state::GateOutcome::TimedOut,
            log: path("/home/u/tasks/gate.log"),
        };
        let cases: Vec<(Reply, serde_json::Value)> = vec![
            (
                Reply::Started {
                    team: team_id(1),
                    tasks: vec![task(1), task(2)],
                },
                json!({"kind": "started", "team": TEAM, "tasks": ["task-20261111-153001-k3x9", TASK]}),
            ),
            (Reply::Teams { teams: vec![] }, json!({"kind": "teams", "teams": []})),
            (
                Reply::Team {
                    team: Box::new(state.team(&team_id(1)).unwrap().clone()),
                },
                json!({"kind": "team", "team": serde_json::to_value(state.team(&team_id(1)).unwrap()).unwrap()}),
            ),
            (
                Reply::Task {
                    team: team_id(1),
                    role: found.role,
                    member: Box::new(found.member.clone()),
                },
                json!({"kind": "task", "team": TEAM, "role": "member", "member": serde_json::to_value(found.member).unwrap()}),
            ),
            (
                Reply::Applied { tasks: vec![task(2)] },
                json!({"kind": "applied", "tasks": [TASK]}),
            ),
            (
                Reply::Gate { result: gate.clone() },
                json!({"kind": "gate", "result": serde_json::to_value(&gate).unwrap()}),
            ),
            (
                Reply::Subscribed { teams: vec![] },
                json!({"kind": "subscribed", "teams": []}),
            ),
        ];
        for (reply, expected) in cases {
            let value = serde_json::to_value(&reply).unwrap();

            assert_eq!(value, expected);
            assert_eq!(serde_json::from_value::<Reply>(value).unwrap(), reply);
        }
    }

    #[test]
    fn a_recorded_gate_reply_carries_the_tree_and_the_result_and_reads_back_equal() {
        let record = GateRecord {
            tree: crate::state::GateTree::NotClean,
            result: GateResult {
                head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
                finished_at: at(60),
                outcome: crate::state::GateOutcome::Passed,
                log: path("/home/u/tasks/gate-1.log"),
            },
        };
        let reply = Reply::GateRecorded { record: record.clone() };

        let value = serde_json::to_value(&reply).unwrap();

        assert_eq!(
            value,
            json!({"kind": "gate_recorded", "record": serde_json::to_value(&record).unwrap()})
        );
        assert_eq!(value["record"]["tree"], json!("not_clean"));
        assert_eq!(serde_json::from_value::<Reply>(value).unwrap(), reply);
    }

    #[test]
    fn a_reply_with_a_bad_part_is_refused() {
        for value in [
            json!({"kind": "started", "team": TASK, "tasks": []}),
            json!({"kind": "started", "team": TEAM}),
            json!({"kind": "teams"}),
            json!({"kind": "subscribed"}),
            json!({"kind": "team", "team": {}}),
            json!({"kind": "applied", "tasks": [TEAM]}),
            json!({"kind": "rebooted"}),
        ] {
            assert_refused::<Reply>(&value);
        }
    }

    #[test]
    fn a_response_is_ok_with_a_reply_or_an_error_with_a_failure() {
        let ok = Response::ok(RequestId(3), Reply::Subscribed { teams: vec![] });
        let failed = Response::error(
            RequestId(4),
            Failure {
                kind: FailureKind::NotFound,
                message: String::from("no task task-x; see agent status"),
            },
        );

        assert_eq!(ok.id, RequestId(3));
        assert_eq!(
            ok.result,
            Outcome::Ok {
                reply: Reply::Subscribed { teams: vec![] }
            }
        );
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"protocol": 2, "id": 3, "result": {"status": "ok", "reply": {"kind": "subscribed", "teams": []}}})
        );
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            json!({"protocol": 2, "id": 4, "result": {"status": "error", "error": {"kind": "not_found", "message": "no task task-x; see agent status"}}})
        );
        for response in [ok, failed] {
            let value = serde_json::to_value(&response).unwrap();
            assert_eq!(serde_json::from_value::<Response>(value).unwrap(), response);
        }
    }

    #[test]
    fn every_kind_of_failure_has_its_text() {
        for (kind, text) in [
            (FailureKind::Invalid, "invalid"),
            (FailureKind::NotFound, "not_found"),
            (FailureKind::Conflict, "conflict"),
            (FailureKind::Refused, "refused"),
            (FailureKind::Internal, "internal"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(text));
            assert_eq!(serde_json::from_value::<FailureKind>(json!(text)).unwrap(), kind);
        }
        assert_refused::<FailureKind>(&json!("oops"));
        assert_refused::<Response>(&json!({"protocol": 2, "id": 1, "result": {"status": "maybe"}}));
        assert_refused::<Response>(
            &json!({"protocol": 1, "id": 1, "result": {"status": "ok", "reply": {"kind": "subscribed", "teams": []}}}),
        );
        assert_refused::<Failure>(&json!({"kind": "invalid"}));
    }
}
