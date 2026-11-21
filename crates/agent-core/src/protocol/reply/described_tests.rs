//! The summary of a team that says what the team's task is, in words, is made on purpose; the plain summary says none.

use super::*;
use crate::state::fixtures::{LIST, team};

/// The sample team with the header line `Short desc: Given words` in its task list.
fn team_with_a_description() -> crate::state::Team {
    let mut value = serde_json::to_value(team(1)).unwrap();
    value["task_list"] = serde_json::json!({
        "text": LIST.replacen("\n\n", "\nShort desc: Given words\n\n", 1),
        "allow_lines": "permissions",
        "short_desc_header": true
    });
    serde_json::from_value(value).unwrap()
}

#[test]
fn the_described_summary_of_a_team_without_a_line_has_the_derived_description() {
    let summary = TeamSummary::described(&team(1));

    assert_eq!(summary.short_desc.as_deref(), Some("Build the thing."));
}

#[test]
fn the_described_summary_of_a_team_with_a_line_has_that_description() {
    let summary = TeamSummary::described(&team_with_a_description());

    assert_eq!(summary.short_desc.as_deref(), Some("Given words"));
}

#[test]
fn the_plain_summary_of_a_team_says_no_description() {
    assert_eq!(TeamSummary::from(&team_with_a_description()).short_desc, None);
}

#[test]
fn the_described_summary_differs_from_the_plain_one_in_the_description_only() {
    let team = team(1);

    let mut described = TeamSummary::described(&team);
    described.short_desc = None;

    assert_eq!(described, TeamSummary::from(&team));
}
