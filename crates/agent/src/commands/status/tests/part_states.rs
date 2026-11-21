//! The state of every part, in the full view and in the running view.

use super::*;
use crate::fixtures::{at, team_state};

/// The sample team with its item 1.1 split into `members` parts, each depending on the parts before it.
fn split_team(members: usize) -> agent_core::state::Team {
    let mut team = team_state().teams()[0].clone();
    let item = "1.1".parse().unwrap();
    team.open_round("round-20261111-153101-q1w2".parse().unwrap(), Some(item), at(1))
        .unwrap();
    team.advance_item(
        item,
        agent_core::state::ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/1.1".parse().unwrap(),
        },
    )
    .unwrap();
    let parts: Vec<serde_json::Value> = (0..members)
        .map(|position| {
            let depends: Vec<String> = (0..position)
                .filter_map(agent_core::state::split::PartKey::at_position)
                .map(|key| key.to_string())
                .collect();
            serde_json::json!({
                "title": "A part", "delivers": "An independent guide", "checks": "Read the guide",
                "depends_on": depends,
            })
        })
        .collect();
    team.activate_split(serde_json::from_value(serde_json::json!({"parent":"1.1","parts":parts})).unwrap())
        .unwrap();
    team
}

/// A round of item 1.1, open or closed, bound to the part `part` if any.
fn round(id: &str, part: Option<&str>, life: &str) -> serde_json::Value {
    let mut round = serde_json::json!({
        "id": id, "item": "1.1", "opened_at": at(2),
        "state": if life == "open" {
            serde_json::json!({"status": "open"})
        } else {
            serde_json::json!({"status": "closed", "at": at(3), "how": life})
        },
    });
    match part {
        Some(key) if life == "merged" => {
            round["part"] = serde_json::json!({"status": "merged", "key": key, "pr": crate::fixtures::pr(3)});
        }
        Some(key) => round["part"] = serde_json::json!({"status": "working", "key": key}),
        None => {}
    }
    round
}

/// The sample summary with the rounds `rounds` instead of its own.
fn with_rounds(team: &agent_core::state::Team, rounds: serde_json::Value) -> TeamSummary {
    let mut wire = serde_json::to_value(TeamSummary::from(team)).unwrap();
    wire["rounds"] = rounds;
    serde_json::from_value(wire).unwrap()
}

/// A state of a part and its text.
type PartRow = (String, &'static str);

/// The state of every part of the plan of item 1.1, as `splits::states` decides it.
fn states_of(team: &TeamSummary) -> Vec<PartRow> {
    let states = super::splits::states(team);
    states
        .first()
        .map(|(_, parts)| {
            parts
                .iter()
                .map(|(id, state)| {
                    let text = match state {
                        super::splits::PartState::Done => "done",
                        super::splits::PartState::InProgress => "in progress",
                        super::splits::PartState::Ready => "to do",
                        super::splits::PartState::WaitingFor(_) => "waiting",
                        super::splits::PartState::WaitingForTier => "waiting for a tier",
                    };
                    (id.to_string(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

// Every part takes the same state from the full team in the full view and in the running view: a merged part is done,
// and a part whose round was closed without a merge is not.
#[test]
fn a_part_takes_its_state_from_the_full_team() {
    let cases = [
        (
            "merged by its round",
            with_rounds(
                &split_team(2),
                serde_json::json!([
                    round("round-20261111-153102-q1w2", Some("a"), "merged"),
                    round("round-20261111-153103-q1w2", Some("b"), "open"),
                ]),
            ),
            vec![("1.1a".to_owned(), "done"), ("1.1b".to_owned(), "in progress")],
        ),
        (
            "its round closed without a merge",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "abandoned")]),
            ),
            vec![("1.1a".to_owned(), "to do"), ("1.1b".to_owned(), "waiting")],
        ),
        (
            "a dependency not merged",
            with_rounds(
                &split_team(3),
                serde_json::json!([round("round-20261111-153102-q1w2", None, "open")]),
            ),
            vec![
                ("1.1a".to_owned(), "to do"),
                ("1.1b".to_owned(), "waiting"),
                ("1.1c".to_owned(), "waiting"),
            ],
        ),
    ];
    for (case, team, expected) in cases {
        assert_eq!(states_of(&team), expected, "{case}");
    }
}

/// The part lines of a rendered status.
fn part_lines(shown: &str) -> Vec<String> {
    shown
        .lines()
        .filter(|line| line.starts_with("    1.1"))
        .map(std::string::ToString::to_string)
        .collect()
}

// The running view is the full team with only its open round lines: a merged part is done in both, and the item
// whose parts are all merged shows no part line in the running view and all done in the full view.
#[test]
fn the_running_view_prints_the_parts_of_the_full_team() {
    let merged = with_rounds(
        &split_team(2),
        serde_json::json!([
            round("round-20261111-153102-q1w2", Some("a"), "merged"),
            round("round-20261111-153103-q1w2", Some("b"), "merged"),
            {
                "id": "round-20261111-153104-q1w2", "item": "1.2", "opened_at": at(4),
                "state": {"status": "open"},
            },
        ]),
    );

    let full = render(std::slice::from_ref(&merged));
    let running = render_running(std::slice::from_ref(&merged));

    assert_eq!(part_lines(&full), ["    1.1a  done  A part", "    1.1b  done  A part"]);
    assert!(part_lines(&running).is_empty(), "{running}");
    assert!(
        running.lines().all(|line| !line.contains("round-20261111-153102-q1w2")),
        "{running}"
    );
    assert!(
        running.lines().any(|line| line.contains("round-20261111-153104-q1w2")),
        "{running}"
    );
}

/// The part lines of both views of `team`, which must be `expected`.
fn assert_both_views(team: &TeamSummary, expected: &[&str], case: &str) {
    for (view, shown) in [
        ("full", render(std::slice::from_ref(team))),
        ("running", render_running(std::slice::from_ref(team))),
    ] {
        assert_eq!(part_lines(&shown), expected, "{case}: {view} view");
    }
}

// The same part has the same state in both views, whichever of the six situations it is in.
#[test]
fn every_part_has_the_same_state_in_both_views() {
    let cases: [(&str, TeamSummary, &[&str]); 6] = [
        (
            "merged by its round",
            with_rounds(
                &split_team(2),
                serde_json::json!([
                    round("round-20261111-153102-q1w2", Some("a"), "merged"),
                    round("round-20261111-153103-q1w2", Some("b"), "open"),
                ]),
            ),
            &["    1.1a  done  A part", "    1.1b  in progress  A part"],
        ),
        (
            "its round open",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "open")]),
            ),
            &["    1.1a  in progress  A part", "    1.1b  waiting for 1.1a  A part"],
        ),
        (
            "its round closed without a merge",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "abandoned")]),
            ),
            &["    1.1a  to do  A part", "    1.1b  waiting for 1.1a  A part"],
        ),
        (
            "nothing started and no dependency, after another part merged",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", None, "open")]),
            ),
            &["    1.1a  to do  A part", "    1.1b  waiting for 1.1a  A part"],
        ),
        (
            "a dependency not merged",
            with_rounds(
                &split_team(3),
                serde_json::json!([round("round-20261111-153102-q1w2", None, "open")]),
            ),
            &[
                "    1.1a  to do  A part",
                "    1.1b  waiting for 1.1a  A part",
                "    1.1c  waiting for 1.1a, 1.1b  A part",
            ],
        ),
        (
            "two dependencies of which one is merged",
            with_rounds(
                &split_team(3),
                serde_json::json!([
                    round("round-20261111-153102-q1w2", Some("a"), "merged"),
                    round("round-20261111-153103-q1w2", Some("b"), "open"),
                ]),
            ),
            &[
                "    1.1a  done  A part",
                "    1.1b  in progress  A part",
                "    1.1c  waiting for 1.1b  A part",
            ],
        ),
    ];
    for (case, team, expected) in cases {
        assert_both_views(&team, expected, case);
    }
}
