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
