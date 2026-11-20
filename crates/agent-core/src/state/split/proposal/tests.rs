use super::*;
use serde_json::{Value, json};

fn part() -> Value {
    json!({"title":"Résumé","delivers":"Reviewed images","checks":"Reviewer counts the images","depends_on":[],"estimate":{"amount":12.5,"unit":"images"}})
}
fn proposal() -> Value {
    json!({"parent":"1.1","scope":{"kind":"unchanged"},"parts":[part(),part()]})
}

#[test]
fn proposal_reader_accepts_number_units_and_defaults_tiers() {
    for tier in [None, Some("investigator"), Some("normal"), Some("worker")] {
        let mut value = proposal();
        if let Some(tier) = tier {
            value["parts"][0]["tier"] = json!(tier);
        }
        let parsed = read_proposal(&value.to_string()).unwrap();
        assert_eq!(parsed.parent(), "1.1".parse().unwrap());
        assert_eq!(parsed.scope(), &ScopeChange::Unchanged);
        assert_eq!(parsed.parts()[0].estimate.as_ref().unwrap().unit, "images");
        assert_eq!(
            parsed.parts()[0].tier,
            tier.map_or(Tier::Normal, |tier| tier.parse().unwrap())
        );
    }
}

#[test]
fn proposal_reader_rejects_each_missing_field_and_unknown_fields() {
    for field in ["parent", "scope", "parts"] {
        let mut value = proposal();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            matches!(read_proposal(&value.to_string()), Err(ReadProposalError::Json { .. })),
            "{field}"
        );
    }
    for field in ["title", "delivers", "checks", "depends_on", "estimate"] {
        let mut value = proposal();
        value["parts"][0].as_object_mut().unwrap().remove(field);
        let err = read_proposal(&value.to_string()).unwrap_err();
        if field == "estimate" {
            assert!(matches!(err, ReadProposalError::Estimate { part: 1 }));
        } else {
            assert!(matches!(err, ReadProposalError::Json { .. }), "{field}");
        }
    }
    for path in ["root", "part", "estimate", "scope"] {
        let mut value = proposal();
        match path {
            "root" => value["extra"] = json!(true),
            "part" => value["parts"][0]["extra"] = json!(true),
            "estimate" => value["parts"][0]["estimate"]["extra"] = json!(true),
            "scope" => value["scope"]["extra"] = json!(true),
            _ => unreachable!(),
        }
        assert!(
            matches!(read_proposal(&value.to_string()), Err(ReadProposalError::Json { .. })),
            "{path}"
        );
    }
}

#[test]
fn proposal_reader_rejects_bad_estimates_tiers_scope_and_oversized_input() {
    for bad in [json!("12"), json!(null), json!(true)] {
        let mut value = proposal();
        value["parts"][0]["estimate"]["amount"] = bad;
        assert!(matches!(
            read_proposal(&value.to_string()),
            Err(ReadProposalError::Json { .. })
        ));
    }
    for field in ["amount", "unit"] {
        let mut value = proposal();
        value["parts"][0]["estimate"].as_object_mut().unwrap().remove(field);
        assert!(matches!(
            read_proposal(&value.to_string()),
            Err(ReadProposalError::Json { .. })
        ));
    }
    for estimate in [json!({"amount":-1,"unit":"lines"}), json!({"amount":1,"unit":" "})] {
        let mut value = proposal();
        value["parts"][0]["estimate"] = estimate;
        assert!(matches!(
            read_proposal(&value.to_string()),
            Err(ReadProposalError::Estimate { part: 1 })
        ));
    }
    let mut value = proposal();
    value["parts"][0]["tier"] = json!("expert");
    assert!(matches!(
        read_proposal(&value.to_string()),
        Err(ReadProposalError::Json { .. })
    ));
    for kind in ["less", "more", "different"] {
        let mut value = proposal();
        value["scope"] = json!({"kind":kind,"reason":"Changed deliverables"});
        assert!(matches!(
            read_proposal(&value.to_string()).unwrap().scope(),
            ScopeChange::Less { .. } | ScopeChange::More { .. } | ScopeChange::Different { .. }
        ));
        value["scope"]["reason"] = json!(" ");
        assert!(matches!(
            read_proposal(&value.to_string()),
            Err(ReadProposalError::ScopeReason)
        ));
        value["scope"].as_object_mut().unwrap().remove("reason");
        assert!(matches!(
            read_proposal(&value.to_string()),
            Err(ReadProposalError::Json { .. })
        ));
    }
    assert!(matches!(
        read_proposal(&" ".repeat(crate::questions::MAX_QUESTIONS_BYTES + 1)),
        Err(ReadProposalError::TooLarge)
    ));
}

#[test]
fn legacy_approvals_and_new_requirements_survive_round_trip_without_changing_legacy_output() {
    let legacy = json!({"parent":"1.1","parts":[
        {"title":"Draft","delivers":"Doc","checks":"Review","depends_on":[]},
        {"title":"Publish","delivers":"Bundle","checks":"Inspect","depends_on":["a"]}
    ]});
    for extended in [false, true] {
        let mut input = legacy.clone();
        if extended {
            input["parts"][0]["estimate"] = json!({"amount":0,"unit":"pages"});
            input["parts"][0]["tier"] = json!("investigator");
        }
        let answer = crate::questions::Verdict::from_text(&format!("split\n```json\n{input}\n```"))
            .unwrap()
            .unwrap();
        let plan = crate::questions::split::read_split(&answer).unwrap().unwrap();
        let wanted = if extended {
            Requirements {
                estimate: Some(Estimate {
                    amount: 0.into(),
                    unit: "pages".into(),
                }),
                tier: Tier::Investigator,
            }
        } else {
            Requirements::default()
        };
        assert_eq!(plan.requirements("a".to_owned().try_into().unwrap()), Some(wanted));
        assert_eq!(
            plan.requirements("b".to_owned().try_into().unwrap()),
            Some(Requirements::default())
        );
        assert_eq!(plan.requirements("z".to_owned().try_into().unwrap()), None);
        assert_eq!(serde_json::to_value(&plan).unwrap(), input);
        let restored: SplitPlan = serde_json::from_value(input).unwrap();
        assert_eq!(restored, plan);
    }
    let plan = read_proposal(&proposal().to_string()).unwrap().plan().unwrap();
    assert_eq!(
        plan.requirements("a".to_owned().try_into().unwrap())
            .unwrap()
            .estimate
            .unwrap()
            .unit,
        "images"
    );
}

#[test]
fn serde_cannot_bypass_the_proposal_field_invariants() {
    let expected = read_proposal(&proposal().to_string()).expect("valid proposal");
    assert_eq!(
        serde_json::from_value::<Proposal>(proposal()).expect("deserialize"),
        expected
    );
    for value in [
        {
            let mut value = proposal();
            value["parts"][0].as_object_mut().unwrap().remove("estimate");
            value
        },
        {
            let mut value = proposal();
            value["parts"][0]["estimate"]["amount"] = json!(-1);
            value
        },
        {
            let mut value = proposal();
            value["scope"] = json!({"kind":"less","reason":" "});
            value
        },
    ] {
        assert_eq!(
            serde_json::from_value::<Proposal>(value)
                .expect_err("same boundary")
                .classify(),
            serde_json::error::Category::Data
        );
    }
}
