//! Builders for the tests of the state: names, IDs, times and a small team.

use std::fmt::Debug;
use std::num::NonZeroU32;

use serde::de::DeserializeOwned;
use serde_json::error::Category;

use super::{
    Crew, Member, MergeMode, Participant, PrNumber, PullRequest, Review, ReviewOutcome, TaskListText, Team, Timestamp,
};
use crate::config::{AbsolutePath, RepoName};
use crate::ids::{AgentName, BranchName, InvestigatorName, RunId, TaskId, TeamId};

/// The task list of the sample team: one feature with an item done already and one open, a second that waits for it.
pub(crate) const LIST: &str = "\
Coordinator: inv-opus

Build the thing.

1. First feature
[x] 1.1 Done long ago
    Nothing to do.
[ ] 1.2 The next item
    Do it.
    Expected result:
    - it works

2. Second feature
Depends on: 1
[ ] 2.1 Another item
    Do that.
";

/// The ID of task number `n` (0 to 59).
pub(crate) fn task(n: u32) -> TaskId {
    format!("task-20261111-1530{n:02}-k3x9").parse().unwrap()
}

/// The ID of run number `n` (0 to 59).
pub(crate) fn run(n: u32) -> RunId {
    format!("run-20261111-1530{n:02}-r7q2").parse().unwrap()
}

/// The ID of team number `n` (0 to 59).
pub(crate) fn team_id(n: u32) -> TeamId {
    format!("team-20261111-1530{n:02}-t5m8").parse().unwrap()
}

/// An agent name.
pub(crate) fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

/// An investigator name.
pub(crate) fn investigator(name: &str) -> InvestigatorName {
    name.parse().unwrap()
}

/// A branch name.
pub(crate) fn branch(name: &str) -> BranchName {
    name.parse().unwrap()
}

/// A normal agent as a participant.
pub(crate) fn normal(name: &str) -> Participant {
    Participant::Normal(agent(name))
}

/// A worker as a participant.
pub(crate) fn worker(name: &str) -> Participant {
    Participant::Worker(agent(name))
}

/// An investigator as a participant.
pub(crate) fn inv(name: &str) -> Participant {
    Participant::Investigator(investigator(name))
}

/// A member of task `n`.
pub(crate) fn member(n: u32, agent: Participant) -> Member {
    Member::new(task(n), agent)
}

/// `seconds` after 19:15:00 UTC on 2026-11-11.
pub(crate) fn at(seconds: u32) -> Timestamp {
    let start: Timestamp = "2026-11-11T19:15:00Z".parse().unwrap();
    Timestamp::from_utc(start.as_utc() + time::Duration::seconds(i64::from(seconds))).unwrap()
}

/// An absolute path.
pub(crate) fn path(text: &str) -> AbsolutePath {
    text.parse().unwrap()
}

/// A question number or any other non-zero number.
pub(crate) fn nz(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

/// A pull request numbered `number`, opened at 10 seconds.
pub(crate) fn pr(number: u32) -> PullRequest {
    PullRequest {
        number: PrNumber::new(nz(number)),
        branch: branch("agents/codex-1/item-1-2"),
        opened_at: at(10),
    }
}

/// A review by `reviewer`, whose task is `n`, asked at 20 seconds.
pub(crate) fn review(reviewer: &str, n: u32, outcome: ReviewOutcome) -> Review {
    Review {
        reviewer: investigator(reviewer),
        task: task(n),
        requested_at: at(20),
        outcome,
    }
}

/// The sample task list.
pub(crate) fn list() -> TaskListText {
    TaskListText::new(LIST.to_owned()).unwrap()
}

/// The repository name the sample team works on.
pub(crate) fn repo() -> RepoName {
    "team-agents".parse().unwrap()
}

/// A team of `inv-opus` (task 1, the coordinator), `codex-1` (task 2, a member) and `worker-1` (task 3, a worker),
/// with team ID `n`, started at 0 seconds, in autopilot.
pub(crate) fn team(n: u32) -> Team {
    team_of(n, 1)
}

/// The team of [`team`] with ID `n` whose tasks are `first_task`, the next and the one after.
pub(crate) fn team_of(n: u32, first_task: u32) -> Team {
    let crew = Crew::team(
        member(first_task, inv("inv-opus")),
        vec![member(first_task + 1, normal("codex-1"))],
        vec![member(first_task + 2, worker("worker-1"))],
    )
    .unwrap();
    Team::new(team_id(n), repo(), at(0), MergeMode::Autopilot, list(), crew)
}

/// Asserts that `value` is refused as a `T` because its data is not valid for it, not because it is no JSON.
pub(crate) fn assert_refused<T: DeserializeOwned + Debug>(value: &serde_json::Value) {
    let err = serde_json::from_value::<T>(value.clone()).unwrap_err();
    assert_eq!(err.classify(), Category::Data, "{value}: {err}");
}
