//! A small world for the tests of rounds through the public API: three teams in one state, and builders of the IDs,
//! times and phases they use.

use std::num::NonZeroU32;

use agent_core::ids::{AgentName, InvestigatorName, ItemId, RoundId, RunId, TaskId, TeamId};
use agent_core::state::{
    Crew, Member, MergeMode, Participant, PrNumber, PullRequest, Review, ReviewOutcome, State, TaskListText, Team,
    TeamMembers, Timestamp,
};

/// The task list of team 1: item 1.1 is done already, 1.2 and 1.3 are open, and 2.1 waits for the first feature.
pub(crate) const LIST: &str = "\
Build the thing.

1. First feature
[x] 1.1 Done long ago
    Nothing to do.
[ ] 1.2 The next item
    Do it.
[ ] 1.3 The one after it
    Do that.

2. Second feature
Depends on: 1
[ ] 2.1 Another item
    Do more.
";

/// The task list of team 2: one item.
pub(crate) const ONE: &str = "Fix the typo.\n\n1. Typo\n[ ] 1.1 Fix it\n    Change \"teh\" to \"the\".\n";

/// The task list of team 3: no item at all.
pub(crate) const NONE: &str = "Tidy up the notes directory.\n";

/// The team ID `n` (1 to 3).
pub(crate) fn team_id(n: u32) -> TeamId {
    format!("team-20261111-1930{n:02}-t5m8").parse().unwrap()
}

/// The task ID `n` (0 to 59).
pub(crate) fn task(n: u32) -> TaskId {
    format!("task-20261111-1930{n:02}-k3x9").parse().unwrap()
}

/// The run ID `n` (0 to 59).
pub(crate) fn run(n: u32) -> RunId {
    format!("run-20261111-1930{n:02}-r7q2").parse().unwrap()
}

/// The round ID `n` (0 to 59).
pub(crate) fn round(n: u32) -> RoundId {
    format!("round-20261111-1930{n:02}-q1w2").parse().unwrap()
}

/// The item `text`.
pub(crate) fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

/// `seconds` after 19:30:00 UTC on 2026-11-11.
pub(crate) fn at(seconds: u32) -> Timestamp {
    let start: Timestamp = "2026-11-11T19:30:00Z".parse().unwrap();
    Timestamp::from_utc(start.as_utc() + time::Duration::seconds(i64::from(seconds))).unwrap()
}

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn investigator(name: &str) -> InvestigatorName {
    name.parse().unwrap()
}

/// The pull request `number`, opened at 10 seconds.
pub(crate) fn pr(number: u32) -> PullRequest {
    PullRequest {
        number: PrNumber::new(NonZeroU32::new(number).unwrap()),
        branch: "agents/codex-1/item-1-2".parse().unwrap(),
        opened_at: at(10),
    }
}

/// A review by `reviewer` whose task is `n`, asked at 20 seconds.
pub(crate) fn review(reviewer: &str, n: u32, outcome: ReviewOutcome) -> Review {
    Review {
        reviewer: investigator(reviewer),
        task: task(n),
        requested_at: at(20),
        outcome,
    }
}

/// Team 1 (autopilot): `inv-opus` coordinates `codex-1` and `worker-1` (tasks 1, 2 and 3), and `inv-codex` and
/// `inv-gemini` (tasks 7 and 8) review.
fn team_one() -> Team {
    let crew = Crew::team(TeamMembers {
        coordinator: Member::new(task(1), Participant::Investigator(investigator("inv-opus"))),
        members: vec![Member::new(task(2), Participant::Normal(agent("codex-1")))],
        workers: vec![Member::new(task(3), Participant::Worker(agent("worker-1")))],
    })
    .unwrap();
    Team::new(
        team_id(1),
        "team-agents".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(LIST.to_owned()).unwrap(),
        crew,
    )
}

/// A solo team `n` of `claude-1` whose task is `first`, with the task list `list`, started at 0 seconds.
fn solo(n: u32, first: u32, list: &str, merge: MergeMode) -> Team {
    let lead = Member::new(task(first), Participant::Normal(agent("claude-1")));
    Team::new(
        team_id(n),
        "toy-repo".parse().unwrap(),
        at(0),
        merge,
        TaskListText::new(list.to_owned()).unwrap(),
        Crew::solo(lead).unwrap(),
    )
}

/// A state with team 1, team 2 (solo, `--no-merge`, one item) and team 3 (solo, a list without items), none of which
/// has a round.
pub(crate) fn state() -> State {
    let mut state = State::new();
    state.insert_team(team_one()).unwrap();
    state.insert_team(solo(2, 11, ONE, MergeMode::NoMerge)).unwrap();
    state.insert_team(solo(3, 21, NONE, MergeMode::Autopilot)).unwrap();
    for (n, name) in [(7, "inv-codex"), (8, "inv-gemini")] {
        state
            .add_reviewer(
                &team_id(1),
                Member::new(task(n), Participant::Investigator(investigator(name))),
            )
            .unwrap();
    }
    state
}
