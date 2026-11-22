//! The short view of `agent status`: the teams with their rounds, their tasks that are alive and a count of their tasks,
//! and the full view as before.

use std::num::NonZeroU32;

use agent_core::ids::RoundId;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::state::Round;
use agent_core::status::TaskStatus;
use time::UtcOffset;

use super::{Detail, View, render, render_in};
use crate::fixtures::{at, task, team_id, team_state};

/// The header line of the sample team: its items, with no description.
const HEADER: &str =
    "team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done";

/// The lines of the four tasks of the sample team while they are queued, as both views write them.
const QUEUED: &str = "  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
";

/// The lines of the eight tasks of [`team_with_all_statuses`]: the four of the sample team, then its coordinator four
/// times more, one in each status.
const EVERY_STATUS: &str = "  task-20261111-153001-k3x9  coordinator  inv-opus   running
  task-20261111-153002-k3x9  member       claude-1   waiting
  task-20261111-153003-k3x9  worker       glm-1      questions
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
  task-20261111-153001-k3x9  coordinator  inv-opus   paused
  task-20261111-153001-k3x9  coordinator  inv-opus   stopped
  task-20261111-153001-k3x9  coordinator  inv-opus   failed
  task-20261111-153001-k3x9  coordinator  inv-opus   done
";

/// The sample team of the status tests, with no description.
fn sample() -> TeamSummary {
    TeamSummary::from(&team_state().teams()[0])
}

/// The sample team written in the short view, every round text left out.
fn short(teams: &[TeamSummary]) -> String {
    render_in(
        teams,
        View::Full,
        Detail::Short,
        &|_| None,
        &|_| String::new(),
        Zone::Fixed(UtcOffset::UTC),
    )
}

/// Sets the status of the member at `index` of `team`.
fn set_status(team: &mut TeamSummary, index: usize, status: TaskStatus) {
    team.members[index].status = status;
}

/// Asks the question number `number` of the member at `index` of `team`.
fn ask(team: &mut TeamSummary, index: usize, number: u32) {
    team.members[index].open_question = NonZeroU32::new(number);
    team.members[index].status = TaskStatus::Questions;
}

#[test]
fn a_team_of_queued_tasks_lists_them_and_counts_them() {
    assert_eq!(short(&[sample()]), format!("{HEADER}\n{QUEUED}  tasks: 4 queued\n"));
}

#[test]
fn a_task_that_waits_for_an_answer_is_named_with_its_question_number() {
    let mut team = sample();
    ask(&mut team, 1, 2);

    assert_eq!(
        short(&[team]),
        format!(
            "{HEADER}\n  \
             task-20261111-153001-k3x9  coordinator  inv-opus   queued\n  \
             task-20261111-153002-k3x9  member       claude-1   questions, questions number 2 wait for answers\n  \
             task-20261111-153003-k3x9  worker       glm-1      queued\n  \
             task-20261111-153004-k3x9  reviewer     inv-codex  queued\n  \
             tasks: 1 with questions, 3 queued; questions number 2 wait for answers (claude-1, {})\n",
            task(2)
        )
    );
}

#[test]
fn each_status_is_counted_in_its_own_word_in_a_fixed_order() {
    let mut team = sample();
    set_status(&mut team, 0, TaskStatus::Paused);
    set_status(&mut team, 1, TaskStatus::Running);
    set_status(&mut team, 2, TaskStatus::Waiting);
    set_status(&mut team, 3, TaskStatus::Done);

    assert_eq!(
        short(&[team]),
        format!(
            "{HEADER}\n  \
             task-20261111-153001-k3x9  coordinator  inv-opus   paused\n  \
             task-20261111-153002-k3x9  member       claude-1   running\n  \
             task-20261111-153003-k3x9  worker       glm-1      waiting\n  \
             tasks: 1 running, 1 waiting, 1 paused, 1 done\n"
        )
    );
}

#[test]
fn the_failed_and_stopped_tasks_are_counted_as_such() {
    let mut team = sample();
    set_status(&mut team, 0, TaskStatus::Stopped);
    set_status(&mut team, 1, TaskStatus::Failed);
    set_status(&mut team, 2, TaskStatus::Queued);
    set_status(&mut team, 3, TaskStatus::Queued);

    assert_eq!(
        short(&[team]),
        format!(
            "{HEADER}\n  \
             task-20261111-153001-k3x9  coordinator  inv-opus   stopped\n  \
             task-20261111-153002-k3x9  member       claude-1   failed\n  \
             task-20261111-153003-k3x9  worker       glm-1      queued\n  \
             task-20261111-153004-k3x9  reviewer     inv-codex  queued\n  \
             tasks: 2 queued, 1 stopped, 1 failed\n"
        )
    );
}

#[test]
fn every_question_that_waits_is_named_and_the_others_are_not() {
    let mut team = sample();
    ask(&mut team, 1, 2);
    ask(&mut team, 2, 3);

    let text = short(&[team]);

    assert_eq!(
        text,
        format!(
            "{HEADER}\n  \
             task-20261111-153001-k3x9  coordinator  inv-opus   queued\n  \
             task-20261111-153002-k3x9  member       claude-1   questions, questions number 2 wait for answers\n  \
             task-20261111-153003-k3x9  worker       glm-1      questions, questions number 3 wait for answers\n  \
             task-20261111-153004-k3x9  reviewer     inv-codex  queued\n  \
             tasks: 2 with questions, 2 queued; questions number 2 wait for answers (claude-1, {member}); \
             questions number 3 wait for answers (glm-1, {worker})\n",
            member = task(2),
            worker = task(3),
        )
    );
}

#[test]
fn a_short_team_has_a_line_of_each_task_and_no_plan_step() {
    let mut team = sample();
    ask(&mut team, 1, 1);
    team.task_plans.insert(task(2), Vec::new());

    let text = short(&[team]);

    assert_eq!(
        text.lines().filter(|line| line.starts_with("  task-")).count(),
        4,
        "{text}"
    );
    assert!(!text.contains("] "), "{text}");
}

#[test]
fn a_description_ends_the_line_of_the_team_in_the_short_view_too() {
    let mut team = sample();
    team.short_desc = Some(String::from("Ship the guide"));

    assert_eq!(
        short(&[team]),
        format!("{HEADER}  \"Ship the guide\"\n{QUEUED}  tasks: 4 queued\n")
    );
}

#[test]
fn a_round_is_listed_with_what_it_waits_for_and_the_tasks_and_the_count_follow_it() {
    let mut state = team_state();
    let round: RoundId = "round-20261111-153001-q1w2".parse().unwrap();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(round.clone(), Some("1.1".parse().unwrap()), at(10))
        .unwrap();
    let team = TeamSummary::from(&state.teams()[0]);
    let rounds = |round: &Round| format!(", waits for claude-1 {}", round.id());

    let text = render_in(
        &[team],
        View::Running,
        Detail::Short,
        &|_| None,
        &rounds,
        Zone::Fixed(UtcOffset::UTC),
    );

    assert!(
        text.contains(&format!(
            "  round-20261111-153001-q1w2  item 1.1  open, waits for claude-1 round-20261111-153001-q1w2\n{QUEUED}  tasks: 4 queued\n"
        )),
        "{text}"
    );
}

#[test]
fn the_full_view_is_the_text_as_it_was_before_the_short_view() {
    let teams = vec![sample()];

    assert_eq!(
        render_in(
            &teams,
            View::Full,
            Detail::Tasks,
            &|_| None,
            &|_| String::new(),
            Zone::Fixed(UtcOffset::UTC),
        ),
        render(&teams)
    );
}

#[test]
fn a_team_without_tasks_says_so_in_its_count() {
    let mut team = sample();
    team.members.clear();

    assert_eq!(short(&[team]), format!("{HEADER}\n  tasks: none\n"));
}

/// One row of the short-view table test.
struct ShortCase {
    /// Case description.
    name: &'static str,
    /// Teams to render.
    teams: Vec<TeamSummary>,
    /// What follows each round line.
    round_suffix: fn(&Round) -> String,
    /// Expected rendered output.
    expected: String,
}

fn no_round_suffix(_: &Round) -> String {
    String::new()
}

fn waits_for_agent(_: &Round) -> String {
    String::from(", waits for claude-1")
}

fn waits_for_human(_: &Round) -> String {
    String::from(", waits for an answer of the human")
}

fn team_with_rounds(second: bool) -> TeamSummary {
    let r1: RoundId = "round-20261111-153001-q1w2".parse().unwrap();
    let r2: RoundId = "round-20261111-153002-q1w2".parse().unwrap();
    let mut state = team_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(r1, Some("1.1".parse().unwrap()), at(10))
        .unwrap();
    if second {
        state
            .team_mut(&team_id(0))
            .unwrap()
            .open_round(r2, Some("1.2".parse().unwrap()), at(20))
            .unwrap();
    }
    TeamSummary::from(&state.teams()[0])
}

fn team_with_all_statuses() -> TeamSummary {
    let mut all = sample();
    while all.members.len() < 8 {
        let extra = all.members[0].clone();
        all.members.push(extra);
    }
    for (index, status) in [
        TaskStatus::Running,
        TaskStatus::Waiting,
        TaskStatus::Questions,
        TaskStatus::Queued,
        TaskStatus::Paused,
        TaskStatus::Stopped,
        TaskStatus::Failed,
        TaskStatus::Done,
    ]
    .into_iter()
    .enumerate()
    {
        set_status(&mut all, index, status);
    }
    all
}

fn team_with_split(base: &TeamSummary) -> TeamSummary {
    let mut split_wire = serde_json::to_value(base).unwrap();
    split_wire["splits"] = serde_json::json!([{
        "parent": "1.1",
        "approved_by": "harness",
        "parts": [
            {"title": "First", "delivers": "Guide", "checks": "Read", "depends_on": []},
            {"title": "Second", "delivers": "Bundle", "checks": "Read", "depends_on": ["a"]}
        ]
    }]);
    serde_json::from_value(split_wire).unwrap()
}

#[test]
fn short_team_table_covers_rounds_waits_questions_splits_descriptions_and_empty() {
    let r1: RoundId = "round-20261111-153001-q1w2".parse().unwrap();
    let r2: RoundId = "round-20261111-153002-q1w2".parse().unwrap();
    let one_round = team_with_rounds(false);
    let two_rounds = team_with_rounds(true);

    let mut tier_wait = sample();
    tier_wait
        .tier_waits
        .insert("1.1".parse().unwrap(), agent_core::kinds::Tier::Investigator);

    let split_team = team_with_split(&one_round);

    let mut with_question = sample();
    ask(&mut with_question, 1, 4);

    let mut with_desc = sample();
    with_desc.short_desc = Some(String::from("Short status by default"));

    let cases = [
        ShortCase {
            name: "no team at all",
            teams: Vec::new(),
            round_suffix: no_round_suffix,
            expected: String::from("no teams\n"),
        },
        ShortCase {
            name: "one round waiting for named agent",
            teams: vec![one_round.clone()],
            round_suffix: waits_for_agent,
            expected: format!("{HEADER}\n  {r1}  item 1.1  open, waits for claude-1\n{QUEUED}  tasks: 4 queued\n"),
        },
        ShortCase {
            name: "one round waiting for the human",
            teams: vec![one_round],
            round_suffix: waits_for_human,
            expected: format!(
                "{HEADER}\n  {r1}  item 1.1  open, waits for an answer of the human\n{QUEUED}  tasks: 4 queued\n"
            ),
        },
        ShortCase {
            name: "two rounds",
            teams: vec![two_rounds],
            round_suffix: no_round_suffix,
            expected: format!("{HEADER}\n  {r1}  item 1.1  open\n  {r2}  item 1.2  open\n{QUEUED}  tasks: 4 queued\n"),
        },
        ShortCase {
            name: "item waiting for a tier",
            teams: vec![tier_wait],
            round_suffix: no_round_suffix,
            expected: format!("{HEADER}\n    1.1  waiting for investigator agent\n{QUEUED}  tasks: 4 queued\n"),
        },
        ShortCase {
            name: "team with split parts omits part lines in short view",
            teams: vec![split_team],
            round_suffix: no_round_suffix,
            expected: format!("{HEADER}\n  {r1}  item 1.1  open\n{QUEUED}  tasks: 4 queued\n"),
        },
        ShortCase {
            name: "team with a task that has questions",
            teams: vec![with_question],
            round_suffix: no_round_suffix,
            expected: format!(
                "{HEADER}\n  \
                 task-20261111-153001-k3x9  coordinator  inv-opus   queued\n  \
                 task-20261111-153002-k3x9  member       claude-1   questions, questions number 4 wait for answers\n  \
                 task-20261111-153003-k3x9  worker       glm-1      queued\n  \
                 task-20261111-153004-k3x9  reviewer     inv-codex  queued\n  \
                 tasks: 1 with questions, 3 queued; questions number 4 wait for answers (claude-1, {})\n",
                task(2)
            ),
        },
        ShortCase {
            name: "tasks in every status in the count",
            teams: vec![team_with_all_statuses()],
            round_suffix: no_round_suffix,
            expected: format!(
                "{HEADER}\n{EVERY_STATUS}  tasks: 1 running, 1 waiting, 1 with questions, 1 queued, 1 paused, 1 stopped, 1 failed, 1 done\n"
            ),
        },
        ShortCase {
            name: "team without short description",
            teams: vec![sample()],
            round_suffix: no_round_suffix,
            expected: format!("{HEADER}\n{QUEUED}  tasks: 4 queued\n"),
        },
        ShortCase {
            name: "team with short description",
            teams: vec![with_desc],
            round_suffix: no_round_suffix,
            expected: format!("{HEADER}  \"Short status by default\"\n{QUEUED}  tasks: 4 queued\n"),
        },
    ];

    for case in cases {
        let actual = render_in(
            &case.teams,
            View::Full,
            Detail::Short,
            &|_| None,
            &case.round_suffix,
            Zone::Fixed(UtcOffset::UTC),
        );
        assert_eq!(actual, case.expected, "{}", case.name);
    }
}
