//! Builders of states for the tests of the daemon.

use agent_core::config::RepoName;
use agent_core::ids::{AgentName, InvestigatorName, RunId, TaskId, TeamId};
use agent_core::state::{
    Crew, Member, MergeMode, Participant, PrNumber, PullRequest, Review, ReviewOutcome, State, TaskListText, Team,
    TeamMembers, Timestamp,
};

/// The task list of the sample teams: one feature of two items.
pub(crate) const LIST: &str = "\
Build the thing.

1. First feature
[ ] 1.1 The first item
    Do it.
    Expected result:
    - it works
    Checks:
    - run it

[ ] 1.2 The second item
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

/// `seconds` after 19:15:00 UTC on 2026-11-11.
pub(crate) fn at(seconds: u32) -> Timestamp {
    let start: Timestamp = "2026-11-11T19:15:00Z".parse().unwrap();
    Timestamp::from_utc(start.as_utc() + time::Duration::seconds(i64::from(seconds))).unwrap()
}

/// A normal agent.
pub(crate) fn normal(name: &str) -> Participant {
    Participant::Normal(name.parse::<AgentName>().unwrap())
}

/// A worker.
pub(crate) fn worker(name: &str) -> Participant {
    Participant::Worker(name.parse::<AgentName>().unwrap())
}

/// An investigator.
pub(crate) fn investigator(name: &str) -> Participant {
    Participant::Investigator(name.parse::<InvestigatorName>().unwrap())
}

/// A member of task `n`.
pub(crate) fn member(n: u32, agent: Participant) -> Member {
    Member::new(task(n), agent)
}

/// The repository the sample teams work on.
pub(crate) fn repo() -> RepoName {
    "team-agents".parse().unwrap()
}

/// A state with a solo team: `claude-1`, task 1.
pub(crate) fn solo_state() -> State {
    let crew = Crew::solo(member(1, normal("claude-1"))).unwrap();
    let team = Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(LIST.to_owned()).unwrap(),
        crew,
    );
    let mut state = State::new();
    state.insert_team(team).unwrap();
    state
}

/// A state with a team of a coordinator (task 1, `inv-opus`), a member (task 2, `claude-1`), a worker (task 3,
/// `glm-1`) and a reviewer (task 4, `inv-codex`).
pub(crate) fn team_state() -> State {
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, investigator("inv-opus")),
        members: vec![member(2, normal("claude-1"))],
        workers: vec![member(3, worker("glm-1"))],
    })
    .unwrap();
    let team = Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(LIST.to_owned()).unwrap(),
        crew,
    );
    let mut state = State::new();
    state.insert_team(team).unwrap();
    state
        .add_reviewer(&team_id(0), member(4, investigator("inv-codex")))
        .unwrap();
    state
}

/// A pull request numbered `number`, opened at 10 seconds on the branch of `codex-1`'s item 1.2.
pub(crate) fn pr(number: u32) -> PullRequest {
    PullRequest {
        number: PrNumber::new(std::num::NonZeroU32::new(number).unwrap()),
        branch: "agents/codex-1/item-1-2".parse().unwrap(),
        opened_at: at(10),
    }
}

/// A pending review by `reviewer`, whose task is `n`, asked at 20 seconds.
pub(crate) fn review(reviewer: &str, n: u32) -> Review {
    Review {
        reviewer: reviewer.parse().unwrap(),
        task: task(n),
        requested_at: at(20),
        outcome: ReviewOutcome::Pending,
    }
}
