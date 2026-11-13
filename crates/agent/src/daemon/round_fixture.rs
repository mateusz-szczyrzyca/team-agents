//! Builders of teams in the middle of a round, for the tests of the autopilot: items in progress, in review, being
//! fixed, and tasks that ran and wait.

use std::num::NonZeroU32;

use agent_core::ids::{BranchName, ItemId};
use agent_core::state::{
    Crew, ItemPhase, MergeMode, PrNumber, PullRequest, Review, ReviewOutcome, RunEnd, TaskListText, Team, TeamMembers,
    Timestamp,
};
use agent_core::status::{ReviewVerdict, RunOutcome, TaskStatus};

use crate::fixtures::{at, investigator, member, normal, repo, run, task, team_id};

/// Two features that do not wait for each other, with one item each, and a third item after the first.
pub(super) const LIST: &str = "\
Build the things.

1. First
[ ] 1.1 One
    Do one.
    Expected result:
    - one works
[ ] 1.2 One more
    Do one more.

2. Second
[ ] 2.1 Two
    Do two.
";

pub(super) fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

pub(super) fn branch(text: &str) -> BranchName {
    text.parse().unwrap()
}

/// The pull request `number` of `head`, opened at 100 seconds.
pub(super) fn pr(number: u32, head: &str) -> PullRequest {
    PullRequest {
        number: PrNumber::new(NonZeroU32::new(number).unwrap()),
        branch: branch(head),
        opened_at: at(100),
    }
}

/// A team on `list` led by the coordinator `inv-opus` (task 1) with the members `claude-1` (task 2) and `claude-2`
/// (task 3), in autopilot.
pub(super) fn led(list: &str) -> Team {
    led_with(list, MergeMode::Autopilot)
}

/// [`led`] with the merge mode `merge`.
pub(super) fn led_with(list: &str, merge: MergeMode) -> Team {
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, investigator("inv-opus")),
        members: vec![member(2, normal("claude-1")), member(3, normal("claude-2"))],
        workers: vec![],
    })
    .unwrap();
    Team::new(
        team_id(0),
        repo(),
        at(0),
        merge,
        TaskListText::new(list.to_owned()).unwrap(),
        crew,
    )
}

/// The branch the coordinator of [`led`] works on.
pub(super) fn lead_branch() -> String {
    format!("agents/inv-opus/{}", task(1))
}

/// Moves the task `n` through `statuses`.
pub(super) fn go(team: &mut Team, n: u32, statuses: &[TaskStatus]) {
    let (_, mut found) = team.find_mut(&task(n)).unwrap();
    for status in statuses {
        found.transition(*status).unwrap();
    }
}

/// Task `n` ran and waits.
pub(super) fn waits(team: &mut Team, n: u32) {
    go(team, n, &[TaskStatus::Running, TaskStatus::Waiting]);
}

/// Task `n` began its run `r` at `started` seconds and it ended at `ended` seconds with `STATUS: DONE`; the task waits.
pub(super) fn ran(team: &mut Team, n: u32, r: u32, started: u32, ended: u32) {
    let (_, mut found) = team.find_mut(&task(n)).unwrap();
    found.begin_run(run(r), at(started), None).unwrap();
    found.transition(TaskStatus::Running).unwrap();
    found
        .end_run(
            at(ended),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .unwrap();
    found.transition(TaskStatus::Waiting).unwrap();
}

/// Gives `id` to `assignee`, on `on`.
pub(super) fn start(team: &mut Team, id: &str, assignee: &str, on: &str) {
    team.item_mut(item(id))
        .unwrap()
        .advance(ItemPhase::InProgress {
            assignee: assignee.parse().unwrap(),
            branch: branch(on),
        })
        .unwrap();
}

/// Puts `id`, which is in progress or being fixed, in review on the pull request `number` of `head`.
pub(super) fn open(team: &mut Team, id: &str, number: u32, head: &str) {
    team.item_mut(item(id))
        .unwrap()
        .advance(ItemPhase::InReview {
            pr: pr(number, head),
            reviews: Vec::new(),
        })
        .unwrap();
}

/// A pending review by `name` in task `n`, asked at `requested` seconds.
pub(super) fn pending(name: &str, n: u32, requested: u32) -> Review {
    Review {
        reviewer: name.parse().unwrap(),
        task: task(n),
        requested_at: at(requested),
        outcome: ReviewOutcome::Pending,
    }
}

/// Asks the reviews of the round of `id`.
pub(super) fn ask(team: &mut Team, id: &str, reviews: Vec<Review>) {
    team.item_mut(item(id)).unwrap().ask_reviews(reviews).unwrap();
}

/// The reviewer of task `n` reports `verdict` on `id` at `seconds`.
pub(super) fn submit(team: &mut Team, id: &str, n: u32, verdict: ReviewVerdict, seconds: u32) {
    team.item_mut(item(id))
        .unwrap()
        .submit_review(&task(n), verdict, at(seconds))
        .unwrap();
}

/// Sends `id`, which is in review with a request for changes, back to `assignee` on `on`, on the pull request `number`
/// of `head`.
pub(super) fn fix(team: &mut Team, id: &str, assignee: &str, on: &str, number: u32, head: &str) {
    team.item_mut(item(id))
        .unwrap()
        .advance(ItemPhase::Fixing {
            assignee: assignee.parse().unwrap(),
            branch: branch(on),
            pr: pr(number, head),
        })
        .unwrap();
}

/// `seconds` as a timestamp, for the expectations.
pub(super) fn secs(seconds: u32) -> Timestamp {
    at(seconds)
}

/// A state with `team` and the investigators `reviewers` (name and task number) as its reviewers.
pub(super) fn state_of(team: Team, reviewers: &[(&str, u32)]) -> agent_core::state::State {
    let id = team.id().clone();
    let mut state = agent_core::state::State::new();
    state.insert_team(team).unwrap();
    for (name, n) in reviewers {
        state.add_reviewer(&id, member(*n, investigator(name))).unwrap();
    }
    state
}

/// In `state`, task `n` began its run `r` at `started` seconds and it ended at `ended` seconds with `STATUS: DONE`; the
/// task waits.
pub(super) fn ran_in(state: &mut agent_core::state::State, n: u32, r: u32, started: u32, ended: u32) {
    let (_, mut found) = state.find_task_mut(&task(n)).unwrap();
    found.begin_run(run(r), at(started), None).unwrap();
    found.transition(TaskStatus::Running).unwrap();
    found
        .end_run(
            at(ended),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .unwrap();
    found.transition(TaskStatus::Waiting).unwrap();
}
