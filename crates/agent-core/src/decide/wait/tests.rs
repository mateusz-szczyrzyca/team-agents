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

#[test]
fn an_item_delivered_by_a_team_pr_waits_for_its_merge_without_a_defect() {
    let waiting = round_wait(Activity::Idle, Some(Obligation::Merge)).unwrap();
    assert_eq!(waiting.text(), "the delivering pull request to merge");
}
// A round whose run is queued waits for that run: it is not a round that waits for nothing.
#[test]
fn a_round_whose_run_is_queued_waits_for_the_run_and_is_not_a_defect() {
    assert_eq!(
        round_wait(Activity::Idle, Some(Obligation::QueuedRun)),
        Some(Waiting::For(Obligation::QueuedRun))
    );
}

// The new wait has a line of its own in the catalog, and the defect keeps its own.
#[test]
fn the_queued_run_and_the_unowned_defect_have_texts_of_their_own() {
    assert_eq!(
        Waiting::For(Obligation::QueuedRun).text(),
        "the queued run of its task to start"
    );
    assert_ne!(Waiting::For(Obligation::QueuedRun).text(), Waiting::Defect.text());
}
