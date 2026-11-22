//! The short view of `agent status`: the teams with their rounds and a count of their tasks, and the full view as before.

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
fn a_team_of_queued_tasks_counts_them_and_lists_none_of_them() {
    assert_eq!(short(&[sample()]), format!("{HEADER}\n  tasks: 4 queued\n"));
}

#[test]
fn a_task_that_waits_for_an_answer_is_named_with_its_question_number() {
    let mut team = sample();
    ask(&mut team, 1, 2);

    assert_eq!(
        short(&[team]),
        format!(
            "{HEADER}\n  tasks: 1 with questions, 3 queued; questions number 2 wait for answers (claude-1, {})\n",
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
        format!("{HEADER}\n  tasks: 1 running, 1 waiting, 1 paused, 1 done\n")
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
        format!("{HEADER}\n  tasks: 2 queued, 1 stopped, 1 failed\n")
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
            "{HEADER}\n  tasks: 2 with questions, 2 queued; questions number 2 wait for answers (claude-1, {member}); \
             questions number 3 wait for answers (glm-1, {worker})\n",
            member = task(2),
            worker = task(3),
        )
    );
}

#[test]
fn a_short_team_has_no_line_of_a_task_and_no_plan_step() {
    let mut team = sample();
    ask(&mut team, 1, 1);
    team.task_plans.insert(task(2), Vec::new());

    let text = short(&[team]);

    assert!(!text.contains("  task-"), "{text}");
    assert!(!text.contains("] "), "{text}");
}

#[test]
fn a_description_ends_the_line_of_the_team_in_the_short_view_too() {
    let mut team = sample();
    team.short_desc = Some(String::from("Ship the guide"));

    assert_eq!(
        short(&[team]),
        format!("{HEADER}  \"Ship the guide\"\n  tasks: 4 queued\n")
    );
}

#[test]
fn a_round_is_listed_with_what_it_waits_for_and_the_count_follows_it() {
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
            "  round-20261111-153001-q1w2  item 1.1  open, waits for claude-1 round-20261111-153001-q1w2\n  tasks: 4 queued\n"
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
