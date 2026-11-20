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
        assert_eq!(parsed.parts()[0].estimate.as_ref().unwrap().unit(), "images");
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
            _ => panic!("unexpected test path {path}"),
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
            Err(ReadProposalError::Json { .. })
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
            Err(ReadProposalError::Json { .. })
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
                estimate: Some(Estimate::new(0.into(), "pages".into()).unwrap()),
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
            .unit(),
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

#[test]
fn estimates_and_scope_reasons_are_validated_wherever_they_are_deserialized() {
    for bad in [
        json!({"amount":-1,"unit":"lines"}),
        json!({"amount":-0.5,"unit":"lines"}),
        json!({"amount":3,"unit":""}),
        json!({"amount":3,"unit":" \t"}),
    ] {
        assert_eq!(
            serde_json::from_value::<Estimate>(bad.clone())
                .expect_err("invalid estimate")
                .classify(),
            serde_json::error::Category::Data,
            "{bad}"
        );
    }
    assert_eq!(
        serde_json::from_value::<Estimate>(json!({"amount":2.5,"unit":"pages"})).unwrap(),
        Estimate::new(serde_json::Number::from_f64(2.5).unwrap(), "pages".into()).unwrap()
    );
    for kind in ["less", "more", "different"] {
        for reason in ["", "  "] {
            assert!(
                serde_json::from_value::<ScopeChange>(json!({"kind":kind,"reason":reason})).is_err(),
                "{kind} {reason:?}"
            );
        }
    }
}

#[test]
fn estimate_constructor_names_what_is_wrong() {
    assert_eq!(Estimate::new((-1).into(), "lines".into()), Err(EstimateError::Amount));
    assert_eq!(Estimate::new(1.into(), "  ".into()), Err(EstimateError::Unit));
    let ok = Estimate::new(0.into(), "images".into()).unwrap();
    assert_eq!((ok.amount(), ok.unit()), (&0.into(), "images"));
}

#[test]
fn plans_and_human_verdicts_reject_invalid_estimates() {
    let part = |estimate: Value| json!({"title":"Draft","delivers":"Doc","checks":"Review","depends_on":[],"estimate":estimate});
    for estimate in [
        json!({"amount":-7,"unit":"  "}),
        json!({"amount":-7,"unit":"lines"}),
        json!({"amount":7,"unit":" "}),
    ] {
        let plan = json!({"parent":"1.1","parts":[part(estimate.clone()), {"title":"Publish","delivers":"Bundle","checks":"Inspect","depends_on":["a"]}]});
        assert!(serde_json::from_value::<SplitPlan>(plan.clone()).is_err(), "{estimate}");
        let verdict = crate::questions::Verdict::from_text(&format!("split\n```json\n{plan}\n```"))
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                crate::questions::split::read_split(&verdict),
                Err(crate::questions::split::ReadSplitError::Json { .. })
            ),
            "{estimate}"
        );
    }
}
