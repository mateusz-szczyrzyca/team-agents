//! The item of a notification is the one the notification itself names, not one inferred from a run.
use agent_core::prompt::Variables;

use super::*;
use crate::fixtures::team_state;

/// The variables `pairs`.
fn values(pairs: &[(&str, &str)]) -> Variables {
    pairs.iter().fold(Variables::new(), |values, (name, value)| {
        values.with(name, *value).unwrap()
    })
}

#[test]
fn an_item_or_a_single_item_in_the_items_is_the_item_of_the_notice() {
    let state = team_state();
    let team = &state.teams()[0];

    for pairs in [
        vec![("item", "1.2")],
        vec![("items", "1.2")],
        vec![("items", " 1.2 ")],
        vec![("items", "1.2, 1.2")],
        vec![("item", "1.2"), ("pr", "7")],
    ] {
        assert_eq!(
            item_of_notice(team, &values(&pairs)),
            Some("1.2".parse().unwrap()),
            "{pairs:?}"
        );
    }
}

#[test]
fn several_items_or_none_or_a_text_that_is_no_item_name_no_item() {
    let state = team_state();
    let team = &state.teams()[0];

    for pairs in [
        vec![("items", "1.2, 2.1")],
        vec![("items", "")],
        vec![("item", "not an item")],
        vec![("items", "1.2, x")],
        vec![("note", "1.2")],
        vec![],
    ] {
        assert_eq!(item_of_notice(team, &values(&pairs)), None, "{pairs:?}");
    }
}

/// The sample state with a round of item 1.2 and item 1.2 in review with pull request 7.
fn state_with_a_round_and_a_pull_request() -> agent_core::state::State {
    use agent_core::state::ItemPhase;

    let mut state = team_state();
    {
        let mut team = state.team_mut(&crate::fixtures::team_id(0)).unwrap();
        team.open_round(
            "round-20261111-153001-q1w2".parse().unwrap(),
            Some("1.2".parse().unwrap()),
            crate::fixtures::at(10),
        )
        .unwrap();
        let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
        item.advance(ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/task-20261111-153002-k3x9-1.2".parse().unwrap(),
        })
        .unwrap();
        item.advance(ItemPhase::InReview {
            pr: crate::fixtures::pr(7),
            reviews: Vec::new(),
        })
        .unwrap();
    }
    state
}

#[test]
fn a_round_names_the_item_the_round_is_for() {
    let state = state_with_a_round_and_a_pull_request();
    let team = &state.teams()[0];

    let found = item_of_notice(team, &values(&[("round", "round-20261111-153001-q1w2")]));
    let several = item_of_notice(
        team,
        &values(&[("rounds", "round-20261111-153001-q1w2, round-20261111-153099-q1w2")]),
    );

    assert_eq!(found, Some("1.2".parse().unwrap()));
    assert_eq!(several, None);
}

#[test]
fn a_pull_request_names_the_item_that_has_it() {
    let state = state_with_a_round_and_a_pull_request();
    let team = &state.teams()[0];

    assert_eq!(
        item_of_notice(team, &values(&[("pr", "7")])),
        Some("1.2".parse().unwrap())
    );
    assert_eq!(item_of_notice(team, &values(&[("pr", "8")])), None);
}
