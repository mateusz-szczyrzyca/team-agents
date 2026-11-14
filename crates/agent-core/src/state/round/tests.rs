use serde_json::json;

use super::*;
use crate::kinds::tests::{assert_keywords, assert_refuses};
use crate::state::fixtures::{assert_refused, at, round_id};

/// A round for item 1.2, opened at 10 seconds.
fn opened() -> Round {
    Round::opened(round_id(1), Some("1.2".parse().unwrap()), at(10))
}

#[test]
fn the_ways_a_round_ends_have_their_texts() {
    let table = [
        (RoundEnd::Merged, "merged"),
        (RoundEnd::Approved, "approved"),
        (RoundEnd::Abandoned, "abandoned"),
    ];

    assert_eq!(RoundEnd::ALL, table.iter().map(|&(how, _)| how).collect::<Vec<_>>());
    assert_keywords(&table);
    assert_refuses("round ending", &table, &["merge", "Merged", "done", "stopped"]);
    assert_eq!(RoundEnd::Abandoned.as_str(), "abandoned");
}

#[test]
fn a_new_round_is_open_with_its_item_and_the_time_it_was_opened() {
    let round = opened();

    assert_eq!(round.id(), &round_id(1));
    assert_eq!(round.item(), Some("1.2".parse().unwrap()));
    assert_eq!(round.opened_at(), at(10));
    assert_eq!(round.life(), &RoundLife::Open);
    assert!(round.is_open());
    assert_eq!(round.ended(), None);
}

#[test]
fn a_round_is_closed_once_with_the_time_and_the_way_it_ended() {
    let mut round = opened();

    round.close(at(20), RoundEnd::Merged).unwrap();

    assert!(!round.is_open());
    assert_eq!(round.ended(), Some(RoundEnd::Merged));
    assert_eq!(
        round.life(),
        &RoundLife::Closed {
            at: at(20),
            how: RoundEnd::Merged
        }
    );
}

#[test]
fn a_round_is_not_closed_before_it_was_opened() {
    let mut round = opened();

    let err = round.close(at(9), RoundEnd::Abandoned).unwrap_err();

    assert_eq!(
        err,
        ClosedBeforeOpened {
            round: round_id(1),
            opened_at: at(10),
            at: at(9)
        }
    );
    assert!(round.is_open());
}

#[test]
fn open_and_closed_rounds_are_written_in_the_form_of_the_state_and_read_back_equal() {
    let open = opened();
    let mut closed = opened();
    closed.close(at(20), RoundEnd::Approved).unwrap();
    let whole_work = Round::opened(round_id(2), None, at(10));

    assert_eq!(
        serde_json::to_value(&open).unwrap(),
        json!({
            "id": "round-20261111-153001-q1w2",
            "item": "1.2",
            "opened_at": "2026-11-11T19:15:10Z",
            "state": {"status": "open"}
        })
    );
    assert_eq!(
        serde_json::to_value(&closed).unwrap()["state"],
        json!({"status": "closed", "at": "2026-11-11T19:15:20Z", "how": "approved"})
    );
    assert_eq!(
        serde_json::to_value(&whole_work).unwrap(),
        json!({"id": "round-20261111-153002-q1w2", "opened_at": "2026-11-11T19:15:10Z", "state": {"status": "open"}})
    );
    for round in [open, closed, whole_work] {
        let value = serde_json::to_value(&round).unwrap();
        assert_eq!(serde_json::from_value::<Round>(value).unwrap(), round);
    }
}

#[test]
fn a_round_is_refused_when_read_if_it_is_not_made_of_what_a_round_has() {
    let good = json!({
        "id": "round-20261111-153001-q1w2",
        "item": "1.2",
        "opened_at": "2026-11-11T19:15:10Z",
        "state": {"status": "closed", "at": "2026-11-11T19:15:20Z", "how": "merged"}
    });
    serde_json::from_value::<Round>(good.clone()).unwrap();
    let mutations = [
        ("id", json!("team-20261111-153001-q1w2")),
        ("id", json!("round-1")),
        ("item", json!("1")),
        ("item", json!(0)),
        ("opened_at", json!("yesterday")),
        ("state", json!({"status": "closed", "at": "2026-11-11T19:15:20Z"})),
        ("state", json!({"status": "closed", "how": "merged"})),
        (
            "state",
            json!({"status": "closed", "at": "2026-11-11T19:15:20Z", "how": "lost"}),
        ),
        (
            "state",
            json!({"status": "closed", "at": "2026-11-11T19:15:09Z", "how": "merged"}),
        ),
        ("state", json!({"status": "open", "at": "2026-11-11T19:15:20Z"})),
        ("state", json!({"status": "paused"})),
        ("state", json!("open")),
        ("extra", json!(true)),
    ];
    for (key, bad) in mutations {
        let mut value = good.clone();
        value[key] = bad;

        assert_refused::<Round>(&value);
    }
    let mut missing = good;
    missing.as_object_mut().unwrap().remove("opened_at");
    assert_refused::<Round>(&missing);
}
