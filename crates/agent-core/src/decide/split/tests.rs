use super::*;
use crate::state::split::PartDefinition;

fn key(letter: &str) -> PartKey {
    letter.to_owned().try_into().expect("key")
}

fn plan() -> SplitPlan {
    SplitPlan::new(
        "1.1".parse().expect("item"),
        [vec![], vec![key("a")], vec![key("b")]]
            .into_iter()
            .map(|depends_on| PartDefinition {
                title: "Part".into(),
                delivers: "Document".into(),
                checks: "Reviewed".into(),
                depends_on,
            })
            .collect(),
    )
    .expect("plan")
}

#[test]
fn next_parts_obeys_merge_dependencies_and_closes_only_after_the_last_part() {
    let plan = plan();
    let ids: Vec<_> = plan.entries().map(|(id, _)| id).collect();
    for (merged, expected) in [
        (vec![], NextParts::Ready(vec![ids[0]])),
        (vec![key("a")], NextParts::Ready(vec![ids[1]])),
        (vec![key("b")], NextParts::Ready(vec![ids[0], ids[2]])),
        (vec![key("a"), key("b")], NextParts::Ready(vec![ids[2]])),
        (vec![key("a"), key("b"), key("c")], NextParts::Complete),
    ] {
        assert_eq!(
            next_parts(&plan, &merged.into_iter().collect()).expect("known progress"),
            expected
        );
    }
}

#[test]
fn next_parts_rejects_unknown_merged_keys_instead_of_losing_progress() {
    assert_eq!(
        next_parts(&plan(), &BTreeSet::from([key("z")])),
        Err(UnknownMergedPart { part: key("z") })
    );
}

/// A round of `item` bound to `part` in the state `state`, which is the JSON of the part's state.
fn round_of(item: &str, id: &str, part: &serde_json::Value, life: &serde_json::Value) -> crate::state::Round {
    serde_json::from_value(serde_json::json!({
        "id": id, "item": item, "opened_at": "2026-11-21T10:00:00Z", "state": life, "part": part,
    }))
    .expect("round")
}

#[test]
fn merged_parts_counts_only_rounds_of_the_item_that_carry_a_part_pull_request() {
    let pr = serde_json::json!({
        "number": 7, "branch": "agents/claude-1/1.1a", "opened_at": "2026-11-21T10:30:00Z",
    });
    let open = serde_json::json!({"status": "open"});
    let closed = serde_json::json!({"status": "closed", "at": "2026-11-21T11:00:00Z", "how": "merged"});
    let given_up = serde_json::json!({"status": "closed", "at": "2026-11-21T11:00:00Z", "how": "abandoned"});
    let rounds = [
        round_of(
            "1.1",
            "round-20261121-100000-aaaa",
            &serde_json::json!({"status": "merged", "key": "a", "pr": pr}),
            &closed,
        ),
        round_of(
            "1.1",
            "round-20261121-100100-bbbb",
            &serde_json::json!({"status": "working", "key": "b"}),
            &given_up,
        ),
        round_of(
            "1.1",
            "round-20261121-100200-cccc",
            &serde_json::json!({"status": "working", "key": "c"}),
            &open,
        ),
        round_of(
            "1.2",
            "round-20261121-100300-dddd",
            &serde_json::json!({"status": "merged", "key": "a", "pr": pr}),
            &closed,
        ),
    ];

    let merged = merged_parts(&rounds, "1.1".parse().expect("item"));

    assert_eq!(merged, BTreeSet::from([key("a")]));
}
