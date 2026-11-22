//! `agent status` says how often the lead returned the item of a task: on the task's line, and on the round's line when
//! the limit is near.

use std::num::NonZeroU32;

use agent_core::protocol::{Returned, TeamSummary};

use super::*;
use crate::fixtures::team_state;

/// The sample team whose member `claude-1` holds item 1.1, which the lead returned `counted` times against `limit`.
fn returned(counted: u32, limit: u32) -> TeamSummary {
    let mut team = TeamSummary::from(&team_state().teams()[0]);
    team.members[1].returned = Some(Returned {
        item: "1.1".parse().unwrap(),
        counted: NonZeroU32::new(counted).unwrap(),
        limit: NonZeroU32::new(limit),
    });
    let mut wire = serde_json::to_value(&team).unwrap();
    wire["rounds"] = serde_json::json!([{
        "id": "round-20261111-153001-q1w2", "item": "1.1", "opened_at": crate::fixtures::at(1),
        "state": {"status": "open"}
    }]);
    serde_json::from_value(wire).unwrap()
}

/// The lines of the rendering of `team` that contain `needle`.
fn lines_with(team: &TeamSummary, needle: &str) -> Vec<String> {
    render(std::slice::from_ref(team))
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_task_line_of_a_returned_item_says_how_often_against_the_limit() {
    assert_eq!(
        lines_with(&returned(3, 5), "claude-1"),
        ["  task-20261111-153002-k3x9  member       claude-1   queued, returned 3 of 5"]
    );
}

#[test]
fn the_task_line_says_only_how_often_when_there_is_no_limit() {
    assert_eq!(
        lines_with(&returned(3, 0), "claude-1"),
        ["  task-20261111-153002-k3x9  member       claude-1   queued, returned 3"]
    );
}

#[test]
fn a_task_nobody_returned_says_nothing_of_it() {
    let team = TeamSummary::from(&team_state().teams()[0]);

    assert_eq!(lines_with(&team, "returned"), Vec::<String>::new());
}

#[test]
fn the_round_line_says_it_only_when_the_limit_is_near() {
    let table = [
        (1, false),
        (2, false),
        (3, false),
        (4, true),
        (5, true),
        // No limit: nothing is near.
        (0, false),
    ];
    for (counted, near) in table {
        let team = if counted == 0 {
            returned(4, 0)
        } else {
            returned(counted, 5)
        };
        let round: Vec<String> = lines_with(&team, "round-20261111-153001-q1w2");
        let expected = if near {
            vec![format!(
                "  round-20261111-153001-q1w2  item 1.1  open, item returned {counted} of 5"
            )]
        } else {
            vec!["  round-20261111-153001-q1w2  item 1.1  open".to_owned()]
        };
        assert_eq!(round, expected, "counted {counted}");
    }
}
