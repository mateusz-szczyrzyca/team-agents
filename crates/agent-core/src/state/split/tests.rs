use super::*;

fn key(raw: &str) -> PartKey {
    PartKey::try_from(raw.to_owned()).unwrap()
}

fn definitions() -> Vec<PartDefinition> {
    vec![
        PartDefinition {
            title: "Draft — część الأولى".to_owned(),
            delivers: "A standalone document".to_owned(),
            checks: "A reviewer confirms its claims".to_owned(),
            depends_on: vec![],
        },
        PartDefinition {
            title: "Publish".to_owned(),
            delivers: "An independently usable release".to_owned(),
            checks: "The release file exists".to_owned(),
            depends_on: vec![key("a")],
        },
    ]
}

#[test]
fn split_accepts_independent_deliverables_with_checks_and_preserves_their_order() {
    let parent = "1.1".parse().unwrap();
    let parts = definitions();
    let plan = SplitPlan::new(parent, parts.clone()).unwrap();
    assert_eq!(plan.parent(), parent);
    assert_eq!(plan.parts(), parts);
    assert_eq!(
        serde_json::from_str::<SplitPlan>(&serde_json::to_string(&plan).unwrap()).unwrap(),
        plan
    );
}

#[test]
fn split_rejects_incomplete_parts_unknown_or_duplicate_dependencies_and_cycles() {
    let parent = "1.1".parse().unwrap();
    for (field, expected) in [
        (PartField::Title, "part a has no title"),
        (PartField::Deliverable, "part a has no deliverable"),
        (PartField::Checks, "part a has no checks"),
    ] {
        let mut parts = definitions();
        let text = match field {
            PartField::Title => &mut parts[0].title,
            PartField::Deliverable => &mut parts[0].delivers,
            PartField::Checks => &mut parts[0].checks,
        };
        *text = " \n\t".to_owned();
        let error = SplitPlan::new(parent, parts).unwrap_err();
        assert_eq!(error, SplitError::Empty { part: key("a"), field });
        assert_eq!(error.to_string(), expected);
    }
    let mut unknown = definitions();
    unknown[1].depends_on = vec![key("c")];
    let error = SplitPlan::new(parent, unknown).unwrap_err();
    assert_eq!(
        error,
        SplitError::UnknownDependency {
            part: key("b"),
            dependency: key("c")
        }
    );
    assert_eq!(error.to_string(), "part b depends on unknown part c");

    let mut duplicate = definitions();
    duplicate[1].depends_on.push(key("a"));
    assert_eq!(
        SplitPlan::new(parent, duplicate),
        Err(SplitError::DuplicateDependency {
            part: key("b"),
            dependency: key("a")
        })
    );
    for dependencies in [vec![key("a")], vec![key("b")]] {
        let mut cycle = definitions();
        cycle[0].depends_on = dependencies;
        let error = SplitPlan::new(parent, cycle).unwrap_err();
        assert_eq!(error, SplitError::Cycle);
        assert_eq!(error.to_string(), "split dependencies contain a cycle");
    }
    for count in [0, 1, 27] {
        let parts = vec![definitions()[0].clone(); count];
        let error = SplitPlan::new(parent, parts).unwrap_err();
        assert_eq!(error, SplitError::Count { count });
        assert_eq!(
            error.to_string(),
            format!("a split needs 2 through 26 parts, got {count}")
        );
    }
}

#[test]
fn part_keys_accept_exactly_one_lowercase_ascii_letter() {
    for raw in ["a", "z"] {
        assert_eq!(key(raw).to_string(), raw);
        assert_eq!(serde_json::from_str::<PartKey>(&format!("{raw:?}")).unwrap(), key(raw));
    }
    for raw in ["", "A", "aa", "1", "ą", " a", "a "] {
        assert_eq!(
            PartKey::try_from(raw.to_owned()),
            Err(InvalidPartKey { raw: raw.to_owned() })
        );
    }
    assert!(matches!("1.1a".parse::<ItemId>(), Err(crate::ids::InvalidItemId::Shape { value }) if value == "1.1a"));
}

#[test]
fn deserializing_a_split_cannot_bypass_validation() {
    let value = serde_json::json!({"parent": "1.1", "parts": definitions()});
    let mut invalid = value.clone();
    invalid["parts"][0]["checks"] = serde_json::json!("");
    assert_eq!(
        serde_json::from_value::<SplitPlan>(invalid)
            .expect_err("blank checks")
            .classify(),
        serde_json::error::Category::Data
    );
    let mut extra = value;
    extra["extra"] = serde_json::json!(true);
    assert_eq!(
        serde_json::from_value::<SplitPlan>(extra)
            .expect_err("unknown field")
            .classify(),
        serde_json::error::Category::Data
    );
}

#[test]
fn parts_are_named_by_the_parent_and_their_declared_order() {
    let parent = "1.1".parse().unwrap();
    let plan = SplitPlan::new(parent, definitions()).unwrap();
    let entries: Vec<_> = plan.entries().collect();
    assert_eq!(
        entries.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>(),
        ["1.1a", "1.1b"]
    );
    assert_eq!(entries[1].0.parent(), parent);
    assert_eq!(entries[1].0.key(), key("b"));
    assert_eq!(entries[1].1, &definitions()[1]);
}
