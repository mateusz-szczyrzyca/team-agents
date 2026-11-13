//! The persistent state of the harness: teams, their members and roles, the task list with the progress of each
//! item, runs, questions, pull requests, reviews and gate results (see `docs/state.md`).
//!
//! The daemon owns one [`State`], saves it as JSON and reads it back; the CLI and the dashboard see parts of it
//! through the control protocol ([`crate::protocol`]). Everything here is pure data with its rules: a value that
//! exists is a valid one, and a text that is not valid is refused when it is read.

mod checked;
mod checks;
#[cfg(test)]
pub(crate) mod fixtures;
mod gate;
mod item;
mod member;
mod question;
mod run;
mod strict;
mod team;
mod values;

pub use checked::CheckedMut;
pub use checks::{Asked, ChecksRecord, ItemChecks, Sent};
pub use gate::{GateOutcome, GateRecord, GateResult, GateTree};
pub use item::{
    AdvanceError, AskReviewsError, Completion, FixPass, FixingWithoutFixPass, InvalidAdvance, InvalidReviews, ItemMut,
    ItemPhase, ItemProgress, NotWorkedOn, PhaseKind, PullRequest, Review, ReviewOutcome, SubmitReviewError,
};
pub use member::{Member, MemberError, MemberMut, Participant, QuestionError};
pub use question::{AnswerBeforeQuestion, AnsweredVia, Question, QuestionState};
pub use run::{Run, RunEnd, RunError, RunState};
pub use team::{
    Crew, CrewError, CrewMut, MergeMode, ReviewerProblem, TaskListText, Team, TeamError, TeamMembers, TeamMut,
};
pub use values::{CommitId, InvalidCommitId, InvalidTimestamp, PrNumber, SessionId, Timestamp};

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::{TaskId, TeamId};
use crate::kinds::Role;

/// The schema version of the state that this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// Largest state text accepted, in bytes: a bound on untrusted input, far above any real state.
pub const MAX_STATE_BYTES: usize = 64 << 20;

/// The schema version of a state file: written as the number [`SCHEMA_VERSION`], and only that number is read.
///
/// A type with one value, so that a state of another version cannot be mistaken for this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion;

version_type!(SchemaVersion, SCHEMA_VERSION, UnsupportedSchema);

/// A state of a schema version this build does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the state has schema version {found}; this build reads version {SCHEMA_VERSION}")]
pub struct UnsupportedSchema {
    /// The version the state declares.
    pub found: u64,
}

/// The state of the harness: every team and, in them, every task.
///
/// Task IDs are unique in the whole state, team IDs too; [`State::insert_team`] and [`State::add_reviewer`] keep
/// them so, and reading a state checks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StateWire")]
pub struct State {
    /// Always [`SCHEMA_VERSION`]; first in the file, so that a reader sees it before anything else.
    schema_version: SchemaVersion,
    /// The teams, oldest first.
    teams: Vec<Team>,
}

/// What a [`State`] is written as, before its checks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    /// See [`State`].
    schema_version: SchemaVersion,
    /// See [`State`].
    teams: Vec<Team>,
}

/// Only the schema version of a state file, read before the rest.
#[derive(Deserialize)]
struct SchemaProbe {
    /// The version the file declares.
    schema_version: u64,
}

/// Why a state could not be read.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The text is longer than [`MAX_STATE_BYTES`].
    #[error("the state has {len} bytes; the limit is {max}")]
    TooLarge {
        /// The text's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The state declares a schema version this build does not read.
    #[error(transparent)]
    UnsupportedSchema(#[from] UnsupportedSchema),
    /// The text is not a state: not JSON, a field missing or unknown, or a value that is not valid.
    #[error("the state is not valid")]
    Malformed {
        /// The parser's reason, with the line and column.
        source: serde_json::Error,
    },
    /// The parts of the state do not fit together.
    #[error(transparent)]
    Invalid(#[from] InvalidState),
}

/// Teams or tasks of a state that clash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidState {
    /// Two teams have one ID.
    #[error("the team {team} appears twice")]
    DuplicateTeam {
        /// The repeated ID.
        team: TeamId,
    },
    /// Two tasks have one ID.
    #[error("the task {task} is in the team {first} and in the team {second}")]
    DuplicateTask {
        /// The repeated ID.
        task: TaskId,
        /// The team that has it first.
        first: TeamId,
        /// The team that has it again; the same team when it has it twice.
        second: TeamId,
    },
}

/// Why a reviewer could not be added to a team.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddReviewerError {
    /// The state has no such team.
    #[error("the state has no team {team}")]
    UnknownTeam {
        /// The team asked for.
        team: TeamId,
    },
    /// The reviewer does not fit the team.
    #[error(transparent)]
    Problem(#[from] ReviewerProblem),
}

/// The state could not be written as text.
#[derive(Debug, thiserror::Error)]
#[error("the state cannot be written as JSON")]
pub struct RenderError {
    /// What the serializer said.
    #[source]
    source: serde_json::Error,
}

/// A task found in a state: its team, its role and its member.
#[derive(Debug, Clone, Copy)]
pub struct TaskRef<'a> {
    /// The team the task is in.
    pub team: &'a Team,
    /// The role of the member in it.
    pub role: Role,
    /// The member.
    pub member: &'a Member,
}

impl State {
    /// A state without teams.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            schema_version: SchemaVersion,
            teams: Vec::new(),
        }
    }

    /// The teams, oldest first.
    #[must_use]
    pub fn teams(&self) -> &[Team] {
        &self.teams
    }

    /// The team `id`.
    #[must_use]
    pub fn team(&self, id: &TeamId) -> Option<&Team> {
        self.teams.iter().find(|team| team.id() == id)
    }

    /// The team `id`, to change it through the checked operations of [`TeamMut`] only.
    ///
    /// The handle does not give out the team as a value to overwrite, so no change can bring in a second team with
    /// one ID or a member that the team's checks did not see. Each of these is refused by the compiler:
    #[doc = include_str!("../tests/docs/team-mut.md")]
    pub fn team_mut(&mut self, id: &TeamId) -> Option<TeamMut<'_>> {
        self.teams.iter_mut().find(|team| team.id() == id).map(TeamMut::new)
    }

    /// The task `task`, with its team and its role.
    #[must_use]
    pub fn find_task(&self, task: &TaskId) -> Option<TaskRef<'_>> {
        self.teams
            .iter()
            .find_map(|team| team.find(task).map(|(role, member)| TaskRef { team, role, member }))
    }

    /// The member of task `task`, to change its task through the checked operations of [`MemberMut`] only; the
    /// member cannot be overwritten with another one:
    #[doc = include_str!("../tests/docs/find-task-mut.md")]
    pub fn find_task_mut(&mut self, task: &TaskId) -> Option<(Role, MemberMut<'_>)> {
        self.teams.iter_mut().find_map(|team| team.find_mut(task))
    }

    /// Adds a team.
    ///
    /// # Errors
    /// [`InvalidState`] when the state has a team with this ID or a task with one of the team's task IDs.
    pub fn insert_team(&mut self, team: Team) -> Result<(), InvalidState> {
        if self.team(team.id()).is_some() {
            return Err(InvalidState::DuplicateTeam {
                team: team.id().clone(),
            });
        }
        for member in team_members(&team) {
            if let Some(owner) = self.find_task(member.task()) {
                return Err(InvalidState::DuplicateTask {
                    task: member.task().clone(),
                    first: owner.team.id().clone(),
                    second: team.id().clone(),
                });
            }
        }
        self.teams.push(team);
        Ok(())
    }

    /// Adds a reviewer to team `team`.
    ///
    /// # Errors
    /// [`AddReviewerError`] when the state has no such team, or the reviewer is not an investigator, is in the team's
    /// crew or among its reviewers already, or its task ID is in use.
    pub fn add_reviewer(&mut self, team: &TeamId, reviewer: Member) -> Result<(), AddReviewerError> {
        if self.team(team).is_none() {
            return Err(AddReviewerError::UnknownTeam { team: team.clone() });
        }
        if self.find_task(reviewer.task()).is_some() {
            return Err(ReviewerProblem::DuplicateTask {
                task: reviewer.task().clone(),
            }
            .into());
        }
        match self.teams.iter_mut().find(|found| found.id() == team) {
            Some(found) => found.add_reviewer(reviewer).map_err(AddReviewerError::from),
            None => Err(AddReviewerError::UnknownTeam { team: team.clone() }),
        }
    }

    /// Reads a state from JSON text.
    ///
    /// # Errors
    /// [`StateError`]: the text is too large, declares another schema version (checked first, so that a state of a
    /// newer version is refused as such and not as malformed), is not a valid state, or has clashing teams or tasks.
    pub fn from_json(text: &str) -> Result<Self, StateError> {
        if text.len() > MAX_STATE_BYTES {
            return Err(StateError::TooLarge {
                len: text.len(),
                max: MAX_STATE_BYTES,
            });
        }
        let probe: SchemaProbe = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
        if probe.schema_version != u64::from(SCHEMA_VERSION) {
            return Err(UnsupportedSchema {
                found: probe.schema_version,
            }
            .into());
        }
        let wire: StateWire = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
        Ok(Self::try_from(wire)?)
    }

    /// The state as JSON text, indented, ending with a newline.
    ///
    /// # Errors
    /// [`RenderError`] when the serializer fails.
    pub fn to_json(&self) -> Result<String, RenderError> {
        let mut text = serde_json::to_string_pretty(self).map_err(|source| RenderError { source })?;
        text.push('\n');
        Ok(text)
    }
}

/// Every member of `team`: its crew, then its reviewers.
fn team_members(team: &Team) -> impl Iterator<Item = &Member> {
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member)
        .chain(team.reviewers())
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<StateWire> for State {
    type Error = InvalidState;

    fn try_from(wire: StateWire) -> Result<Self, Self::Error> {
        let mut teams = HashSet::new();
        let mut owners: HashMap<&TaskId, &TeamId> = HashMap::new();
        for team in &wire.teams {
            if !teams.insert(team.id()) {
                return Err(InvalidState::DuplicateTeam {
                    team: team.id().clone(),
                });
            }
            for member in team_members(team) {
                if let Some(first) = owners.insert(member.task(), team.id()) {
                    return Err(InvalidState::DuplicateTask {
                        task: member.task().clone(),
                        first: first.clone(),
                        second: team.id().clone(),
                    });
                }
            }
        }
        Ok(Self {
            schema_version: wire.schema_version,
            teams: wire.teams,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::error::Category;
    use serde_json::json;

    use super::fixtures::{assert_refused, inv, member, normal, task, team_id, team_of};
    use super::*;
    use crate::state::ItemPhase;
    use crate::status::TaskStatus;

    /// Two teams with nothing in common.
    fn two_teams() -> State {
        let mut state = State::new();
        state.insert_team(team_of(1, 1)).unwrap();
        state.insert_team(team_of(2, 11)).unwrap();
        state
    }

    // ---- building a state ---------------------------------------------------------------------------------------

    #[test]
    fn a_new_state_has_no_teams() {
        let state = State::new();

        assert_eq!(state.teams(), []);
        assert_eq!(state, State::default());
        assert!(state.team(&team_id(1)).is_none());
    }

    #[test]
    fn teams_are_added_in_order_and_found_by_id() {
        let state = two_teams();

        let ids: Vec<&TeamId> = state.teams().iter().map(Team::id).collect();

        assert_eq!(ids, [&team_id(1), &team_id(2)]);
        assert_eq!(state.team(&team_id(2)).map(Team::id), Some(&team_id(2)));
        assert!(state.team(&team_id(3)).is_none());
    }

    #[test]
    fn a_team_can_be_changed_through_the_state() {
        let mut state = two_teams();

        let item = "1.2".parse().unwrap();
        state
            .team_mut(&team_id(2))
            .unwrap()
            .item_mut(item)
            .unwrap()
            .advance(ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();

        let first = state.team(&team_id(1)).unwrap().item(item).unwrap().status();
        let second = state.team(&team_id(2)).unwrap().item(item).unwrap().status();
        assert_eq!(
            (first, second),
            (
                crate::status::ItemStatus::Pending,
                crate::status::ItemStatus::InProgress
            )
        );
        assert!(state.team_mut(&team_id(3)).is_none());
    }

    #[test]
    fn a_team_with_a_known_id_is_refused_and_the_state_stays_as_it_was() {
        let mut state = two_teams();
        let before = state.clone();

        let err = state.insert_team(team_of(2, 21)).unwrap_err();

        assert_eq!(err, InvalidState::DuplicateTeam { team: team_id(2) });
        assert_eq!(state, before);
    }

    #[test]
    fn a_team_with_a_task_id_the_state_has_is_refused_and_the_state_stays_as_it_was() {
        let mut state = two_teams();
        let before = state.clone();

        let err = state.insert_team(team_of(3, 12)).unwrap_err();

        assert_eq!(
            err,
            InvalidState::DuplicateTask {
                task: task(12),
                first: team_id(2),
                second: team_id(3)
            }
        );
        assert_eq!(state, before);
    }

    #[test]
    fn a_task_is_found_with_its_team_and_role_and_its_member_can_be_changed() {
        let mut state = two_teams();
        state.add_reviewer(&team_id(1), member(31, inv("inv-codex"))).unwrap();

        let member = state.find_task(&task(12)).unwrap();
        assert_eq!(
            (member.team.id(), member.role, member.member.agent().name()),
            (&team_id(2), Role::Member, "codex-1")
        );
        assert_eq!(
            state.find_task(&task(31)).map(|t| (t.team.id().clone(), t.role)),
            Some((team_id(1), Role::Reviewer))
        );
        assert!(state.find_task(&task(40)).is_none());
        let (role, mut found) = state.find_task_mut(&task(12)).unwrap();
        assert_eq!(role, Role::Member);
        found.transition(TaskStatus::Running).unwrap();
        assert_eq!(
            state.find_task(&task(12)).map(|t| t.member.status()),
            Some(TaskStatus::Running)
        );
        assert_eq!(
            state.find_task(&task(2)).map(|t| t.member.status()),
            Some(TaskStatus::Queued)
        );
        assert!(state.find_task_mut(&task(40)).is_none());
    }

    #[test]
    fn a_reviewer_is_added_to_the_team_it_reviews() {
        let mut state = two_teams();

        state.add_reviewer(&team_id(2), member(31, inv("inv-codex"))).unwrap();

        assert_eq!(state.team(&team_id(2)).map(|t| t.reviewers().len()), Some(1));
        assert_eq!(state.team(&team_id(1)).map(|t| t.reviewers().len()), Some(0));
    }

    #[test]
    fn a_reviewer_for_an_unknown_team_or_with_a_task_in_use_anywhere_is_refused() {
        let mut state = two_teams();
        state.add_reviewer(&team_id(1), member(31, inv("inv-codex"))).unwrap();
        let before = state.clone();

        assert_eq!(
            state.add_reviewer(&team_id(9), member(32, inv("inv-gemini"))),
            Err(AddReviewerError::UnknownTeam { team: team_id(9) })
        );
        for taken in [12, 31] {
            assert_eq!(
                state.add_reviewer(&team_id(1), member(taken, inv("inv-gemini"))),
                Err(AddReviewerError::Problem(ReviewerProblem::DuplicateTask {
                    task: task(taken)
                })),
                "task {taken}"
            );
        }
        assert_eq!(
            state.add_reviewer(&team_id(1), member(32, normal("claude-1"))),
            Err(AddReviewerError::Problem(ReviewerProblem::NotInvestigator {
                agent: normal("claude-1")
            }))
        );
        assert_eq!(state, before);
    }

    // ---- the schema version -------------------------------------------------------------------------------------

    #[test]
    fn the_schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
        assert_eq!(serde_json::to_value(SchemaVersion).unwrap(), json!(1));
    }

    #[test]
    fn only_the_schema_version_of_this_build_is_read() {
        serde_json::from_value::<SchemaVersion>(json!(1)).unwrap();
        for bad in [
            json!(0),
            json!(2),
            json!(u64::MAX),
            json!(-1),
            json!("1"),
            json!(1.5),
            json!(null),
        ] {
            assert_refused::<SchemaVersion>(&bad.clone());
        }
    }

    // ---- reading and writing ------------------------------------------------------------------------------------

    #[test]
    fn a_state_saved_and_loaded_back_is_equal() {
        let mut state = two_teams();
        state.add_reviewer(&team_id(1), member(31, inv("inv-codex"))).unwrap();
        state
            .find_task_mut(&task(2))
            .unwrap()
            .1
            .transition(TaskStatus::Running)
            .unwrap();

        let text = state.to_json().unwrap();

        assert_eq!(State::from_json(&text).unwrap(), state);
    }

    #[test]
    fn an_empty_state_is_written_with_its_schema_version_first() {
        let text = State::new().to_json().unwrap();

        assert_eq!(text, "{\n  \"schema_version\": 1,\n  \"teams\": []\n}\n");
        assert_eq!(State::from_json(&text).unwrap(), State::new());
    }

    #[test]
    fn a_state_is_written_indented_and_ends_with_a_newline_and_the_schema_version_comes_first() {
        let text = two_teams().to_json().unwrap();

        assert!(text.starts_with("{\n  \"schema_version\": 1,\n  \"teams\": [\n    {\n      \"id\": \"team-"));
        assert!(text.ends_with("}\n"));
        assert!(!text.ends_with("\n\n"));
    }

    #[test]
    fn a_state_of_another_schema_version_is_refused_with_that_version() {
        for found in [0_u64, 2, 99, u64::MAX] {
            let text = format!("{{\"schema_version\": {found}, \"teams\": []}}");

            let err = State::from_json(&text).unwrap_err();

            assert!(
                matches!(&err, StateError::UnsupportedSchema(UnsupportedSchema { found: f }) if *f == found),
                "{found}"
            );
        }
    }

    #[test]
    fn the_version_is_checked_before_the_rest_so_a_newer_state_is_not_called_malformed() {
        for text in [
            "{\"schema_version\": 2, \"teams\": {\"by_id\": 5}}",
            "{\"teams\": [], \"schema_version\": 3, \"future\": true}",
            "{\"schema_version\": 2}",
        ] {
            let err = State::from_json(text).unwrap_err();

            assert!(matches!(err, StateError::UnsupportedSchema(_)), "{text}");
        }
    }

    #[test]
    fn a_text_that_is_not_a_state_is_malformed() {
        for text in [
            "",
            "not json",
            "[]",
            "null",
            "{}",
            "{\"teams\": []}",
            "{\"schema_version\": \"1\", \"teams\": []}",
            "{\"schema_version\": -1, \"teams\": []}",
            "{\"schema_version\": 1.5, \"teams\": []}",
            "{\"schema_version\": 1}",
            "{\"schema_version\": 1, \"teams\": [], \"extra\": 1}",
            "{\"schema_version\": 1, \"teams\": [{}]}",
            "{\"schema_version\": 1, \"teams\": []} trailing",
        ] {
            let err = State::from_json(text).unwrap_err();

            assert!(matches!(err, StateError::Malformed { .. }), "{text:?}: {err:?}");
        }
    }

    #[test]
    fn a_malformed_state_keeps_the_parsers_reason_as_its_source() {
        let err = State::from_json("{\"schema_version\": 1, \"teams\": [{}]}").unwrap_err();

        let source = std::error::Error::source(&err).expect("a source");

        assert!(source.to_string().contains("missing field"), "{source}");
    }

    #[test]
    fn a_state_with_two_teams_of_one_id_or_two_tasks_of_one_id_is_refused_with_both_teams() {
        let one = serde_json::to_value(team_of(1, 1)).unwrap();
        let same_team = serde_json::to_value(team_of(1, 11)).unwrap();
        let same_tasks = serde_json::to_value(team_of(2, 1)).unwrap();
        let state = |teams: &[&serde_json::Value]| json!({"schema_version": 1, "teams": teams}).to_string();

        let teams = State::from_json(&state(&[&one, &same_team])).unwrap_err();
        let tasks = State::from_json(&state(&[&one, &same_tasks])).unwrap_err();

        assert!(matches!(teams, StateError::Invalid(InvalidState::DuplicateTeam { team }) if team == team_id(1)));
        assert!(matches!(
            tasks,
            StateError::Invalid(InvalidState::DuplicateTask { task: t, first, second })
                if t == task(1) && first == team_id(1) && second == team_id(2)
        ));
    }

    #[test]
    fn a_reviewer_with_a_task_of_its_teams_crew_is_refused_when_read() {
        let mut value = serde_json::to_value(team_of(1, 1)).unwrap();
        value["reviewers"] = json!([{
            "task": "task-20261111-153002-k3x9",
            "agent": {"tier": "investigator", "name": "inv-codex"},
            "status": "queued"
        }]);

        let err = State::from_json(&json!({"schema_version": 1, "teams": [value]}).to_string()).unwrap_err();

        assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn a_state_over_the_limit_is_refused_before_it_is_read() {
        let text = " ".repeat(MAX_STATE_BYTES + 1);

        let err = State::from_json(&text).unwrap_err();

        assert!(
            matches!(err, StateError::TooLarge { len, max } if len == MAX_STATE_BYTES + 1 && max == MAX_STATE_BYTES)
        );
    }

    #[test]
    fn a_state_of_exactly_the_limit_is_read_not_refused_for_its_size() {
        let text = " ".repeat(MAX_STATE_BYTES);

        let err = State::from_json(&text).unwrap_err();

        assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn serde_reads_a_state_through_the_same_checks() {
        let good = State::new().to_json().unwrap();
        let other_version = "{\"schema_version\": 2, \"teams\": []}";
        let one = serde_json::to_value(team_of(1, 1)).unwrap();
        let duplicate = json!({"schema_version": 1, "teams": [one, one]}).to_string();

        assert_eq!(serde_json::from_str::<State>(&good).unwrap(), State::new());
        for text in [other_version, duplicate.as_str()] {
            let err = serde_json::from_str::<State>(text).unwrap_err();

            assert_eq!(err.classify(), Category::Data, "{text}");
        }
    }
}
