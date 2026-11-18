//! Which review a report answers, when the items of one pull request stand at different fix passes.
use agent_core::state::{PullRequest, Timestamp};

use super::*;

/// A pull request of the fixture's shape.
fn pull_request() -> PullRequest {
    serde_json::from_value(serde_json::json!({
        "number": 82,
        "branch": "agents/codex-1/item-1-3",
        "opened_at": "2026-11-11T19:40:00Z"
    }))
    .unwrap()
}

#[test]
fn a_report_is_recorded_in_the_pass_of_the_most_fixed_item_when_the_joined_item_comes_first() {
    let requested_at: Timestamp = "2026-11-18T19:31:00Z".parse().unwrap();
    let offered = |item: &str, pass| Offered {
        requested_at,
        item: item.parse().unwrap(),
        pr: pull_request(),
        reviewer: "inv-kimi".parse().unwrap(),
        pass,
    };

    let asked = answer_among(
        vec![offered("2.1", 0), offered("1.3", 1)],
        "2026-11-18T19:32:00Z".parse().unwrap(),
    )
    .unwrap();

    assert_eq!(asked.passes, 1);
    assert_eq!(
        asked.items,
        vec!["2.1".parse::<ItemId>().unwrap(), "1.3".parse().unwrap()]
    );
}
