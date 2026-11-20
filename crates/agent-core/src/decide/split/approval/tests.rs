use super::*;
use crate::state::split::proposal::read_proposal;
use serde_json::{Value, json};

fn input(dependencies: &[&[&str]]) -> Value {
    json!({"parent":"1.1","scope":{"kind":"unchanged"},"parts":dependencies.iter().map(|deps| json!({"title":"Part","delivers":"A checked document","checks":"Reviewer checks examples","depends_on":deps,"estimate":{"amount":1500,"unit":"lines"}})).collect::<Vec<_>>()})
}
fn key(s: &str) -> PartKey {
    s.to_owned().try_into().unwrap()
}
fn decision(value: &Value, policy: Split, corrections: u32) -> ProposalDecision {
    let proposal = read_proposal(&value.to_string()).unwrap();
    decide_proposal(ProposalRead::Parsed(&proposal), policy, "lines", corrections)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table compares all approval outcomes and correction boundaries"
)]
fn approval_table_covers_good_scope_faults_limits_and_correction_budgets() {
    let good = input(&[&[], &["a"]]);
    let proposal = read_proposal(&good.to_string()).unwrap();
    let plan = proposal.plan().unwrap();
    let mut missing = good.clone();
    missing["parts"][1]["checks"] = json!(" \t");
    let mut too_big = good.clone();
    too_big["parts"][1]["estimate"]["amount"] = json!(1500.5);
    let mut other_unit = too_big.clone();
    other_unit["parts"][1]["estimate"]["unit"] = json!("pages");
    let mut zero = good.clone();
    zero["parts"][1]["estimate"]["amount"] = json!(0);
    let mut scope = good.clone();
    scope["scope"] = json!({"kind":"less","reason":"Omit publication"});
    let missing_fault = ProposalFault::MissingChecks { part: key("b") };
    let size_fault = ProposalFault::AboveLimit {
        part: key("b"),
        estimate: Estimate::new(serde_json::Number::from_f64(1500.5).unwrap(), "lines".into()).unwrap(),
        limit: 1500,
    };
    let cases = [
        (
            good.clone(),
            Split::default(),
            0,
            ProposalDecision::Approve(plan.clone()),
        ),
        (good, Split::default(), 2, ProposalDecision::Approve(plan)),
        (
            missing.clone(),
            Split::default(),
            0,
            ProposalDecision::SendBack(missing_fault.clone()),
        ),
        (
            missing.clone(),
            Split::default(),
            1,
            ProposalDecision::SendBack(missing_fault.clone()),
        ),
        (
            missing.clone(),
            Split::default(),
            2,
            ProposalDecision::AskHuman(AskReason::CorrectionsUsed(missing_fault.clone())),
        ),
        (
            missing.clone(),
            Split::default(),
            3,
            ProposalDecision::AskHuman(AskReason::CorrectionsUsed(missing_fault.clone())),
        ),
        (
            missing,
            Split {
                part_limit: 1500,
                corrections: 0,
            },
            0,
            ProposalDecision::AskHuman(AskReason::CorrectionsUsed(missing_fault)),
        ),
        (
            too_big.clone(),
            Split::default(),
            0,
            ProposalDecision::SendBack(size_fault),
        ),
        (
            too_big.clone(),
            Split {
                part_limit: 0,
                corrections: 2,
            },
            0,
            ProposalDecision::Approve(read_proposal(&too_big.to_string()).unwrap().plan().unwrap()),
        ),
        (
            other_unit.clone(),
            Split::default(),
            0,
            ProposalDecision::Approve(read_proposal(&other_unit.to_string()).unwrap().plan().unwrap()),
        ),
        (
            zero.clone(),
            Split::default(),
            0,
            ProposalDecision::Approve(read_proposal(&zero.to_string()).unwrap().plan().unwrap()),
        ),
        (
            scope,
            Split::default(),
            0,
            ProposalDecision::AskHuman(AskReason::ScopeChanged(ScopeChange::Less {
                reason: "Omit publication".into(),
            })),
        ),
    ];
    for (value, policy, corrections, expected) in cases {
        assert_eq!(
            decision(&value, policy, corrections),
            expected,
            "{value}, {policy:?}, corrections={corrections}"
        );
    }
    for (count, expected) in [
        (
            0,
            ProposalDecision::SendBack(ProposalFault::Unreadable { part: Some(key("b")) }),
        ),
        (
            2,
            ProposalDecision::AskHuman(AskReason::CorrectionsUsed(ProposalFault::Unreadable {
                part: Some(key("b")),
            })),
        ),
    ] {
        assert_eq!(
            decide_proposal(
                ProposalRead::Unreadable { part: Some(key("b")) },
                Split::default(),
                "pages",
                count
            ),
            expected
        );
    }
}

#[test]
fn cycle_table_names_a_cycle_member_and_accepts_forward_edges_without_cycles() {
    for (deps, cycle) in [
        (vec![vec![], vec!["b"]], Some("b")),
        (vec![vec![], vec!["c"], vec!["b"]], Some("b")),
        (vec![vec![], vec!["c"], vec!["d"], vec!["b"]], Some("b")),
        (vec![vec![], vec!["b", "c"], vec!["d"], vec!["c"]], Some("b")),
        (vec![vec![], vec!["c"], vec!["a"]], None),
    ] {
        let slices: Vec<_> = deps.iter().map(Vec::as_slice).collect();
        let value = input(&slices);
        let got = decision(&value, Split::default(), 0);
        let expected = match cycle {
            Some(part) => ProposalDecision::SendBack(ProposalFault::Cycle { part: key(part) }),
            None => ProposalDecision::Approve(read_proposal(&value.to_string()).unwrap().plan().unwrap()),
        };
        assert_eq!(got, expected, "{deps:?}");
    }
}

#[test]
fn invalid_whole_plans_never_approve_and_name_parts_where_possible() {
    for (deps, fault) in [
        (
            vec![vec!["b"], vec![]],
            ProposalFault::FirstPartDepends { part: key("a") },
        ),
        (
            vec![vec![], vec!["z"]],
            ProposalFault::UnknownDependency {
                part: key("b"),
                dependency: key("z"),
            },
        ),
        (
            vec![vec![], vec!["a", "a"]],
            ProposalFault::DuplicateDependency {
                part: key("b"),
                dependency: key("a"),
            },
        ),
        (vec![vec![]], ProposalFault::PartCount { count: 1 }),
        (vec![], ProposalFault::PartCount { count: 0 }),
    ] {
        let slices: Vec<_> = deps.iter().map(Vec::as_slice).collect();
        assert_eq!(
            decision(&input(&slices), Split::default(), 0),
            ProposalDecision::SendBack(fault),
            "{deps:?}"
        );
    }
    for (field, fault) in [
        ("title", ProposalFault::EmptyTitle { part: key("b") }),
        ("delivers", ProposalFault::EmptyDeliverable { part: key("b") }),
    ] {
        let mut value = input(&[&[], &[]]);
        value["parts"][1][field] = json!(" ");
        assert_eq!(decision(&value, Split::default(), 0), ProposalDecision::SendBack(fault));
    }
}

#[test]
fn units_match_the_repository_unit_ignoring_case_spaces_and_a_plural() {
    for unit in ["lines", "Lines", " lines ", "LINES\t", "line", "Line "] {
        let mut value = input(&[&[], &["a"]]);
        value["parts"][1]["estimate"] = json!({"amount":9000,"unit":unit});
        assert_eq!(
            decision(&value, Split::default(), 0),
            ProposalDecision::SendBack(ProposalFault::AboveLimit {
                part: key("b"),
                estimate: Estimate::new(9000.into(), unit.into()).unwrap(),
                limit: 1500,
            }),
            "{unit:?}"
        );
    }
    for unit in ["pages", "lineage", "li nes", "linesx"] {
        let mut value = input(&[&[], &["a"]]);
        value["parts"][1]["estimate"] = json!({"amount":9000,"unit":unit});
        assert!(
            matches!(decision(&value, Split::default(), 0), ProposalDecision::Approve(_)),
            "{unit:?}"
        );
    }
}
