//! `agent status` says what each team's task is, in words, on the team's line.

use agent_core::protocol::TeamSummary;

use super::render;
use crate::fixtures::team_state;

/// The sample team with the description `desc`, or none.
fn summary(desc: Option<&str>) -> TeamSummary {
    let mut summary = TeamSummary::from(&team_state().teams()[0]);
    summary.short_desc = desc.map(str::to_owned);
    summary
}

#[test]
fn the_description_ends_the_line_of_the_team() {
    let text = render(&[summary(Some("Fix the typo in the README"))]);

    assert_eq!(
        text.lines().next(),
        Some(
            "team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, \
             0 done  \"Fix the typo in the README\""
        )
    );
}

#[test]
fn a_team_without_a_description_has_the_line_it_had() {
    let text = render(&[summary(None)]);

    assert_eq!(
        text.lines().next(),
        Some(
            "team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done"
        )
    );
}

#[test]
fn each_team_has_its_own_description() {
    let mut other = summary(Some("Second task"));
    other.id = "team-20261111-153059-t5m8".parse().unwrap();

    let text = render(&[summary(Some("First task")), other]);

    let lines: Vec<&str> = text.lines().filter(|line| line.starts_with("team-")).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with("\"First task\""), "{}", lines[0]);
    assert!(lines[1].ends_with("\"Second task\""), "{}", lines[1]);
}
