//! Builders of teams and configurations for the tests of the team's decisions.

use agent_core::config::Config;
use agent_core::ids::{RoundId, TaskId};
use agent_core::state::{Crew, MergeMode, RunEnd, TaskListText, Team, TeamMembers};
use agent_core::status::{RunOutcome, TaskStatus};

use super::{Assigned, start_item};
use crate::fixtures::{at, investigator, member, normal, run, task, team_id, worker};

/// A configuration with three normal agents, two workers (`glm-1` first), three investigators and one repository.
pub(super) const CONFIG: &str = r#"
[agents.claude-1]
cli = "claude"
[agents.claude-2]
cli = "claude"
[agents.codex-1]
cli = "codex"
[agents.glm-1]
cli = "opencode"
[agents.glm-2]
cli = "opencode"
[investigators.inv-opus]
cli = "claude"
[investigators.inv-codex]
cli = "codex"
[investigators.inv-gemini]
cli = "gemini"
[tiers]
normal = ["claude-1", "claude-2", "codex-1"]
workers = ["glm-1", "glm-2"]
[repos.example]
base_branch = "main"
gate = ["true"]
"#;

/// [`CONFIG`], read.
pub(super) fn config() -> Config {
    Config::from_toml_str(CONFIG).unwrap()
}

/// A list of two features that do not wait for each other, with one item each.
pub(super) const PARALLEL: &str = "\
Build the things.

1. First
[ ] 1.1 One
    Do one.

2. Second
[ ] 2.1 Two
    Do two.
";

/// A list of one feature with two items: the second starts when the first is merged.
pub(super) const CHAIN: &str = "\
Build the thing.

1. First
[ ] 1.1 One
    Do one.
[ ] 1.2 Two
    Do two.
";

/// A list whose only item is marked done.
pub(super) const ALL_DONE: &str = "\
Build the thing.

1. First
[x] 1.1 One
    Done already.
";

/// A list without features: a brief and nothing else.
pub(super) const PLAIN: &str = "Fix the typo in the README.\n";

/// A team on `list` with a solo agent (`claude-1`, task 1).
pub(super) fn solo(list: &str) -> Team {
    let crew = Crew::solo(member(1, normal("claude-1"))).unwrap();
    team(list, crew)
}

/// A team on `list`: the coordinator `inv-opus` (task 1), `members` normal members from task 2 on (`claude-1`, ...)
/// and `workers` workers after them (`glm-1`, ...).
pub(super) fn crew_of(list: &str, members: u32, workers: u32) -> Team {
    let people: Vec<_> = (0..members)
        .map(|n| member(2 + n, normal(&format!("claude-{}", 1 + n))))
        .collect();
    let helpers: Vec<_> = (0..workers)
        .map(|n| member(2 + members + n, worker(&format!("glm-{}", 1 + n))))
        .collect();
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, investigator("inv-opus")),
        members: people,
        workers: helpers,
    })
    .unwrap();
    team(list, crew)
}

/// A team of `crew` on `list`, in autopilot.
fn team(list: &str, crew: Crew) -> Team {
    Team::new(
        team_id(0),
        repo_name(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(list.to_owned()).unwrap(),
        crew,
    )
}

/// The repository of the sample teams.
fn repo_name() -> agent_core::config::RepoName {
    "example".parse().unwrap()
}

/// Opens the round of the assigned item and starts it, as the daemon does.
pub(super) fn start(team: &mut Team, given: &Assigned) {
    let position = team.items().iter().position(|item| item.id() == given.item).unwrap();
    let round = RoundId::derive(team.id(), u32::try_from(position).unwrap());
    team.open_round(round, Some(given.item), at(0)).unwrap();
    start_item(team, given).unwrap();
}

/// Moves the task `n` through `statuses`, one after the other.
pub(super) fn go(team: &mut Team, n: u32, statuses: &[TaskStatus]) {
    let (_, mut member) = team.find_mut(&task(n)).unwrap();
    for status in statuses {
        member.transition(*status).unwrap();
    }
}

/// Gives the task `n` a run (number `r`) that ended with `end`, and the statuses `then` after it.
pub(super) fn ran(team: &mut Team, n: u32, r: u32, end: RunEnd, then: &[TaskStatus]) {
    let (_, mut member) = team.find_mut(&task(n)).unwrap();
    member.begin_run(run(r), at(r), None).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member.end_run(at(r + 1), end).unwrap();
    for status in then {
        member.transition(*status).unwrap();
    }
}

/// An end with the final status `outcome`.
pub(super) const fn reported(outcome: RunOutcome) -> RunEnd {
    RunEnd::Reported { outcome }
}

/// The ID of the task `n`.
pub(super) fn task_id(n: u32) -> TaskId {
    task(n)
}
