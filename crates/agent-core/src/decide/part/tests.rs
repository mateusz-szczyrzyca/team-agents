//! Tests for split part assignment pure decisions.

use super::*;
use crate::decide::replacement::ReplacementMode;
use crate::kinds::Tier;
use crate::state::fixtures::{inv, normal};
use crate::status::TaskStatus;

struct TestCase<'a> {
    name: &'static str,
    required: Tier,
    previous: Option<Candidate<'a>>,
    replacements: Vec<Candidate<'a>>,
    lead: Option<Candidate<'a>>,
    expected: PartChoice<'a>,
}

#[test]
fn who_gets_part_prefers_previous_or_replacement() {
    let normal_prev = normal("prev-normal");
    let inv_prev = inv("prev-inv");
    let normal_rep = normal("rep-normal");
    let inv_rep = inv("rep-inv");
    let normal_lead = normal("lead-normal");

    let cases = vec![
        TestCase {
            name: "member free gets the part",
            required: Tier::Normal,
            previous: Some(Candidate::available(&normal_prev)),
            replacements: vec![Candidate::available(&normal_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::PreviousMember(&normal_prev),
        },
        TestCase {
            name: "stopped member yields to fitting replacement",
            required: Tier::Normal,
            previous: Some(Candidate {
                agent: &normal_prev,
                status: TaskStatus::Stopped,
                handed_over: false,
                busy: false,
            }),
            replacements: vec![Candidate::available(&normal_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Replacement(&normal_rep),
        },
        TestCase {
            name: "failed member yields to fitting replacement",
            required: Tier::Normal,
            previous: Some(Candidate {
                agent: &normal_prev,
                status: TaskStatus::Failed,
                handed_over: false,
                busy: false,
            }),
            replacements: vec![Candidate::available(&normal_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Replacement(&normal_rep),
        },
        TestCase {
            name: "handed over member yields to fitting replacement",
            required: Tier::Normal,
            previous: Some(Candidate {
                agent: &normal_prev,
                status: TaskStatus::Waiting,
                handed_over: true,
                busy: false,
            }),
            replacements: vec![Candidate::available(&normal_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Replacement(&normal_rep),
        },
        TestCase {
            name: "underqualified member yields to qualified replacement",
            required: Tier::Investigator,
            previous: Some(Candidate::available(&normal_prev)),
            replacements: vec![Candidate::available(&inv_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Replacement(&inv_rep),
        },
        TestCase {
            name: "busy qualified previous yields to qualified replacement",
            required: Tier::Investigator,
            previous: Some(Candidate {
                agent: &inv_prev,
                status: TaskStatus::Waiting,
                handed_over: false,
                busy: true,
            }),
            replacements: vec![Candidate::available(&inv_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Replacement(&inv_rep),
        },
        TestCase {
            name: "no previous round member with fitting replacement chooses replacement",
            required: Tier::Normal,
            previous: None,
            replacements: vec![Candidate::available(&normal_rep)],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Replacement(&normal_rep),
        },
    ];

    for case in cases {
        let actual = who_gets_part(case.required, case.previous, &case.replacements, case.lead);
        assert_eq!(actual, case.expected, "case: {}", case.name);
    }
}

#[test]
fn who_gets_part_falls_back_to_lead_or_nobody() {
    let normal_prev = normal("prev-normal");
    let inv_prev = inv("prev-inv");
    let normal_rep = normal("rep-normal");
    let inv_rep = inv("rep-inv");
    let normal_lead = normal("lead-normal");
    let inv_lead = inv("lead-inv");

    let cases = vec![
        TestCase {
            name: "stopped member with no fitting replacement yields to lead",
            required: Tier::Normal,
            previous: Some(Candidate {
                agent: &normal_prev,
                status: TaskStatus::Stopped,
                handed_over: false,
                busy: false,
            }),
            replacements: vec![],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Lead(&normal_lead),
        },
        TestCase {
            name: "stopped member with underqualified lead yields nobody",
            required: Tier::Investigator,
            previous: Some(Candidate {
                agent: &normal_prev,
                status: TaskStatus::Stopped,
                handed_over: false,
                busy: false,
            }),
            replacements: vec![],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Nobody,
        },
        TestCase {
            name: "underqualified member with underqualified replacement yields to qualified lead",
            required: Tier::Investigator,
            previous: Some(Candidate::available(&normal_prev)),
            replacements: vec![Candidate::available(&normal_rep)],
            lead: Some(Candidate::available(&inv_lead)),
            expected: PartChoice::Lead(&inv_lead),
        },
        TestCase {
            name: "all fitting candidates busy yields nobody",
            required: Tier::Investigator,
            previous: Some(Candidate {
                agent: &inv_prev,
                status: TaskStatus::Waiting,
                handed_over: false,
                busy: true,
            }),
            replacements: vec![Candidate {
                agent: &inv_rep,
                status: TaskStatus::Waiting,
                handed_over: false,
                busy: true,
            }],
            lead: Some(Candidate {
                agent: &inv_lead,
                status: TaskStatus::Waiting,
                handed_over: false,
                busy: true,
            }),
            expected: PartChoice::Nobody,
        },
        TestCase {
            name: "no previous round member and no replacement chooses lead",
            required: Tier::Normal,
            previous: None,
            replacements: vec![],
            lead: Some(Candidate::available(&normal_lead)),
            expected: PartChoice::Lead(&normal_lead),
        },
    ];

    for case in cases {
        let actual = who_gets_part(case.required, case.previous, &case.replacements, case.lead);
        assert_eq!(actual, case.expected, "case: {}", case.name);
    }
}

#[test]
fn who_gets_part_in_mode_respects_replacement_modes() {
    use crate::decide::replacement::ReplacementMode;

    let normal_prev = normal("prev-normal");
    let normal_rep = normal("rep-normal");
    let normal_lead = normal("lead-normal");

    let stopped_prev = Candidate {
        agent: &normal_prev,
        status: TaskStatus::Stopped,
        handed_over: false,
        busy: false,
    };
    let rep = Candidate::available(&normal_rep);
    let lead = Candidate::available(&normal_lead);

    assert_eq!(
        who_gets_part_in_mode(
            Tier::Normal,
            Some(stopped_prev),
            &[rep],
            Some(lead),
            ReplacementMode::Auto
        ),
        PartChoice::Replacement(&normal_rep)
    );
    assert_eq!(
        who_gets_part_in_mode(
            Tier::Normal,
            Some(stopped_prev),
            &[rep],
            Some(lead),
            ReplacementMode::Lead
        ),
        PartChoice::Lead(&normal_lead)
    );
    assert_eq!(
        who_gets_part_in_mode(
            Tier::Normal,
            Some(stopped_prev),
            &[rep],
            Some(lead),
            ReplacementMode::Off
        ),
        PartChoice::Nobody
    );
}

#[test]
fn replacement_cannot_lower_tier_below_previous_holder() {
    let inv_prev = inv("prev-inv");
    let normal_rep = normal("rep-normal");
    let inv_lead = inv("lead-inv");

    let stopped_prev = Candidate {
        agent: &inv_prev,
        status: TaskStatus::Stopped,
        handed_over: false,
        busy: false,
    };
    let choice = who_gets_part_in_mode(
        Tier::Normal,
        Some(stopped_prev),
        &[Candidate::available(&normal_rep)],
        Some(Candidate::available(&inv_lead)),
        ReplacementMode::Auto,
    );
    assert_eq!(choice, PartChoice::Lead(&inv_lead));
}

#[test]
fn part_choice_explains_outcome_and_reasons() {
    let normal_prev = normal("prev-normal");
    let normal_rep = normal("rep-normal");
    let normal_lead = normal("lead-normal");

    let prev_cand = Candidate {
        agent: &normal_prev,
        status: TaskStatus::Stopped,
        handed_over: false,
        busy: false,
    };

    let choice_prev = PartChoice::PreviousMember(&normal_prev);
    assert_eq!(choice_prev.choice_name(), "previous_member");
    assert_eq!(
        choice_prev.explanation(None),
        "previous member prev-normal is available and qualified"
    );

    let choice_rep = PartChoice::Replacement(&normal_rep);
    assert_eq!(choice_rep.choice_name(), "replacement");
    assert_eq!(
        choice_rep.explanation(Some(&prev_cand)),
        "previous member prev-normal is stopped; replacement rep-normal takes the part"
    );

    let choice_lead = PartChoice::Lead(&normal_lead);
    assert_eq!(choice_lead.choice_name(), "lead");
    assert_eq!(
        choice_lead.explanation(Some(&prev_cand)),
        "previous member prev-normal is stopped; lead lead-normal takes over"
    );

    let choice_nobody = PartChoice::Nobody;
    assert_eq!(choice_nobody.choice_name(), "nobody");
    assert_eq!(
        choice_nobody.explanation(Some(&prev_cand)),
        "previous member prev-normal is stopped; no qualified candidate available"
    );
}
