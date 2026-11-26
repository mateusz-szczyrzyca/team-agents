//! The short view of `agent status` lists the tasks of a team that are alive, each as the full view writes it, without
//! plans and parts, and ends the team with the count of its tasks.

use std::num::NonZeroU32;

use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::protocol::{PlanStepSummary, TeamSummary};
use agent_core::state::{Activity, ActivityEnd, ActivityOutcome, RunEnd};
use agent_core::status::TaskStatus;
use time::UtcOffset;

use super::{Detail, View, render_in};
use crate::fixtures::{at, solo_state, task, team_id, team_state};

/// The header line of the sample team.
const HEADER: &str =
    "team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done";

/// The line of the open round of the sample team.
const ROUND: &str = "  round-20261111-153001-q1w2  item 1.1  open";

/// The first review of `inv-codex` in the task `n` for the round of item 1.1, whose outcome is `outcome`.
fn review(n: u32, outcome: &ActivityOutcome) -> Activity {
    serde_json::from_value(serde_json::json!({
        "investigator": "inv-codex",
        "task": task(n),
        "rounds": ["round-20261111-153001-q1w2"],
        "request": {"kind": "first_review"},
        "asked_at": at(20),
        "outcome": outcome,
    }))
    .unwrap()
}

/// The outcome of a review that ended as `end`, 30 seconds after 19:15:00 UTC.
fn finished(end: ActivityEnd) -> ActivityOutcome {
    ActivityOutcome::Finished { at: at(30), end }
}

/// The sample team with an open round of item 1.1 and a reviewer of each kind:
///
/// | task | who | status | what the team recorded of it |
/// |---|---|---|---|
/// | 1 | the coordinator | running | |
/// | 2 | the member | questions number 2 | |
/// | 3 | the worker | done | |
/// | 4 | a reviewer | running | a review that goes on |
/// | 5 | a reviewer | waiting | a review that gave its report |
/// | 6 | a reviewer | failed | a review that failed |
/// | 7 | a reviewer | stopped | a review the human stopped |
/// | 8 | a reviewer | queued | nothing |
/// | 9 | a reviewer | done | nothing |
fn crew() -> TeamSummary {
    let mut state = team_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(
            "round-20261111-153001-q1w2".parse().unwrap(),
            Some("1.1".parse().unwrap()),
            at(10),
        )
        .unwrap();
    let mut team = TeamSummary::from(&state.teams()[0]);
    for n in 5..=9 {
        let mut reviewer = team.members[3].clone();
        reviewer.task = task(n);
        team.members.push(reviewer);
    }
    for (index, status) in [
        TaskStatus::Running,
        TaskStatus::Questions,
        TaskStatus::Done,
        TaskStatus::Running,
        TaskStatus::Waiting,
        TaskStatus::Failed,
        TaskStatus::Stopped,
        TaskStatus::Queued,
        TaskStatus::Done,
    ]
    .into_iter()
    .enumerate()
    {
        team.members[index].status = status;
    }
    team.members[1].open_question = NonZeroU32::new(2);
    team.activities = vec![
        review(4, &ActivityOutcome::Working),
        review(5, &finished(ActivityEnd::Reported)),
        review(
            6,
            &finished(ActivityEnd::Failed {
                reason: Some(RunEnd::Hung),
            }),
        ),
        review(7, &finished(ActivityEnd::Stopped)),
    ];
    team
}

/// `teams` in `view` and `detail`; a task that runs says `last activity 23:10:11`, the rounds nothing after their lines.
fn shown(teams: &[TeamSummary], view: View, detail: Detail) -> String {
    let said = |_: &TaskId| Some("last activity 23:10:11".to_owned());
    render_in(
        teams,
        view,
        detail,
        &said,
        &|_| String::new(),
        Zone::Fixed(UtcOffset::UTC),
    )
}

/// What the short view writes of [`crew`].
fn crew_short() -> String {
    format!(
        "{HEADER}\n\
         {ROUND}\n  \
         task-20261111-153001-k3x9  coordinator  inv-opus   running, last activity 23:10:11\n  \
         task-20261111-153002-k3x9  member       claude-1   questions, questions number 2 wait for answers\n  \
         task-20261111-153003-k3x9  worker       glm-1      done\n  \
         task-20261111-153004-k3x9  reviewer     inv-codex  working, last activity 23:10:11\n  \
         task-20261111-153005-k3x9  reviewer     inv-codex  done, 1st review finished 2026-11-11 19:15:30\n  \
         task-20261111-153006-k3x9  reviewer     inv-codex  stopped at 2026-11-11 19:15:30, reason: the run \
         showed no activity for too long\n  \
         task-20261111-153007-k3x9  reviewer     inv-codex  stopped at 2026-11-11 19:15:30, reason: the human stopped it\n  \
         task-20261111-153008-k3x9  reviewer     inv-codex  queued\n  \
         task-20261111-153009-k3x9  reviewer     inv-codex  done\n  \
         tasks: 2 running, 1 waiting, 1 with questions, 1 queued, 1 stopped, 1 failed, 2 done; questions number 2 wait \
         for answers (claude-1, task-20261111-153002-k3x9)\n"
    )
}

// Rule: under the rounds of a team the short view writes a line for each of its tasks — the coordinator, the member and
// the worker whatever they do, every reviewer whatever its review's outcome — and then the count of all its tasks.
#[test]
fn the_short_view_lists_the_crew_and_every_reviewer_and_ends_with_the_count() {
    assert_eq!(shown(&[crew()], View::Full, Detail::Short), crew_short());
}

// Rule: `agent status --running` is the same short view.
#[test]
fn the_running_short_view_lists_the_same_tasks() {
    assert_eq!(shown(&[crew()], View::Running, Detail::Short), crew_short());
}

// Rule: the line of a task in the short view is the line the full view writes of it, byte for byte — also when the
// task runs in a new session and was taken over from another agent.
#[test]
fn a_line_of_the_short_view_is_the_line_of_the_full_view() {
    let mut team = crew();
    team.members[0].new_sessions = NonZeroU32::new(2);
    team.members[0].held_by = Some("claude-2".parse().unwrap());
    let teams = [team];

    let short = shown(&teams, View::Full, Detail::Short);
    let full = shown(&teams, View::Full, Detail::Tasks);

    let listed: Vec<&str> = short.lines().filter(|line| line.starts_with("  task-")).collect();
    assert_eq!(listed.len(), 9, "{short}");
    assert_eq!(
        listed[0],
        "  task-20261111-153001-k3x9  coordinator  inv-opus   running, in a new session, for claude-2, last activity \
         23:10:11"
    );
    let full_lines: Vec<&str> = full.lines().collect();
    for line in listed {
        assert!(full_lines.contains(&line), "{line:?} is a line of:\n{full}");
    }
}

// Rule: the plans of the tasks and the parts of a split stay behind `--task`: the short view writes the team, its
// rounds, its tasks and its count, and nothing else.
#[test]
fn the_short_view_has_no_plan_step_and_no_part_of_a_split() {
    let mut wire = serde_json::to_value(crew()).unwrap();
    wire["splits"] = serde_json::json!([{
        "parent": "1.1",
        "approved_by": "harness",
        "parts": [
            {"title": "First", "delivers": "Guide", "checks": "Read", "depends_on": []},
            {"title": "Second", "delivers": "Bundle", "checks": "Read", "depends_on": ["a"]}
        ]
    }]);
    let mut team: TeamSummary = serde_json::from_value(wire).unwrap();
    team.task_plans.insert(
        task(2),
        vec![PlanStepSummary {
            id: "1.1".parse().unwrap(),
            mark: serde_json::from_value(serde_json::json!("x")).unwrap(),
            title: "First step".to_owned(),
            state: None,
            reason: None,
        }],
    );
    let teams = [team];

    let full = shown(&teams, View::Full, Detail::Tasks);
    assert!(full.contains("    [x] 1.1 First step\n"), "{full}");
    assert!(full.contains("    1.1a  "), "{full}");

    assert_eq!(shown(&teams, View::Full, Detail::Short), crew_short());
}

// Rule: a task without a team is listed too, whatever its status.
#[test]
fn a_solo_task_is_listed_also_when_it_is_done_or_stopped() {
    for (status, word) in [
        (TaskStatus::Done, "done"),
        (TaskStatus::Stopped, "stopped"),
        (TaskStatus::Failed, "failed"),
    ] {
        let mut team = TeamSummary::from(&solo_state().teams()[0]);
        team.members[0].status = status;

        let text = shown(&[team], View::Full, Detail::Short);

        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                format!("  task-20261111-153001-k3x9  solo  claude-1  {word}"),
                format!("  tasks: 1 {word}"),
            ],
            "{text}"
        );
    }
}

// Rule: the columns are as wide as the longest role and agent of the tasks of the list, so the lines of the short view
// are those of the full view.
#[test]
fn the_columns_fit_the_longest_agent_name_of_the_listed_tasks() {
    let mut team = crew();
    // The reviewer of task 9 is listed; its agent has the longest name.
    team.members[8].agent = crate::fixtures::investigator("inv-longest-name");

    for detail in [Detail::Short, Detail::Tasks] {
        let text = shown(&[team.clone()], View::Full, detail);

        assert!(
            text.contains("  task-20261111-153009-k3x9  reviewer     inv-longest-name  done\n"),
            "{text}"
        );
        assert!(
            text.contains("  task-20261111-153003-k3x9  worker       glm-1             done\n"),
            "{text}"
        );
    }
}

/// The sample team of four tasks — the coordinator and the member running, the worker and the reviewer done — whose
/// reviewer gave its report at 23:35:56.
fn team_with_a_finished_reviewer() -> TeamSummary {
    let mut team = TeamSummary::from(&team_state().teams()[0]);
    for (index, status) in [
        TaskStatus::Running,
        TaskStatus::Running,
        TaskStatus::Done,
        TaskStatus::Done,
    ]
    .into_iter()
    .enumerate()
    {
        team.members[index].status = status;
    }
    team.activities = vec![review(
        4,
        &ActivityOutcome::Finished {
            at: at(15656),
            end: ActivityEnd::Reported,
        },
    )];
    team
}

// Rule: a reviewer whose review is over stays in the short view, with when its review finished, as it is in the full
// view and in the list of the running teams.
#[test]
fn a_reviewer_whose_review_is_over_is_listed_with_when_it_finished() {
    let line =
        "  task-20261111-153004-k3x9  reviewer     inv-codex  done, 1st review finished 2026-11-11 23:35:56";
    let teams = [team_with_a_finished_reviewer()];

    let tasks = shown(&teams, View::Full, Detail::Tasks);
    assert!(tasks.lines().any(|text| text == line), "{tasks}");
    for view in [View::Full, View::Running] {
        let short = shown(&teams, view, Detail::Short);
        assert!(short.lines().any(|text| text == line), "{short}");
    }
}

// Rule: the count of a team counts the tasks it always counted: listing the finished reviewers changes no word of it.
#[test]
fn listing_finished_reviewers_leaves_the_count_line_as_it_was() {
    let teams = [team_with_a_finished_reviewer()];

    let short = shown(&teams, View::Full, Detail::Short);

    assert_eq!(short.lines().last(), Some("  tasks: 2 running, 2 done"), "{short}");
}
