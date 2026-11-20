//! Every activity and wait reason, including the deliberately unowned round.

use super::{Activity, Obligation, Waiting, round_wait};

#[test]
fn only_idle_open_rounds_wait_and_an_unowned_round_is_a_defect() {
    let obligations = [
        None,
        Some(Obligation::Gate),
        Some(Obligation::Review),
        Some(Obligation::SecondOpinion),
        Some(Obligation::Human),
        Some(Obligation::LeadCorrection),
    ];
    for activity in [Activity::Closed, Activity::Active, Activity::Idle] {
        for obligation in obligations {
            let expected = match activity {
                Activity::Closed | Activity::Active => None,
                Activity::Idle => Some(obligation.map_or(Waiting::Defect, Waiting::For)),
            };
            assert_eq!(
                round_wait(activity, obligation),
                expected,
                "{activity:?}, {obligation:?}"
            );
        }
    }
}

#[test]
fn wait_labels_name_every_owner_and_the_unowned_defect() {
    let cases = [
        (Waiting::For(Obligation::Gate), "a gate of the harness"),
        (Waiting::For(Obligation::Review), "a review"),
        (Waiting::For(Obligation::SecondOpinion), "a second opinion"),
        (Waiting::For(Obligation::Human), "an answer of the human"),
        (Waiting::For(Obligation::LeadCorrection), "the lead's correction"),
        (Waiting::Defect, "nothing: this is a defect"),
    ];
    for (waiting, expected) in cases {
        assert_eq!(waiting.text(), expected);
    }
}

#[test]
fn an_unassigned_review_names_the_missing_free_investigator() {
    assert_eq!(
        Waiting::For(Obligation::FreeInvestigator).text(),
        "a free independent investigator"
    );
}
