//! Every activity and wait reason, including the deliberately unowned round.

use super::{Activity, Obligation, Told, WaitOwner, Waiting, round_wait};
use strum::IntoEnumIterator;

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

// Rule: every wait state has an owner, and the owner is told how: a lead or a reviewer by its prompt, the human by a
// notification, and the harness's own work needs no telling.
const OWNERS: [(Obligation, WaitOwner, Told); 8] = [
    (Obligation::Merge, WaitOwner::Harness, Told::NoOneToTell),
    (Obligation::Gate, WaitOwner::Harness, Told::NoOneToTell),
    (Obligation::Review, WaitOwner::Reviewers, Told::ByPrompt),
    (Obligation::FreeInvestigator, WaitOwner::Harness, Told::NoOneToTell),
    (Obligation::SecondOpinion, WaitOwner::Reviewers, Told::ByPrompt),
    (Obligation::Human, WaitOwner::Human, Told::ByNotification),
    (Obligation::LeadCorrection, WaitOwner::Lead, Told::ByPrompt),
    (Obligation::QueuedRun, WaitOwner::Harness, Told::NoOneToTell),
];

#[test]
fn every_wait_state_has_an_owner_and_a_way_the_owner_is_told() {
    for (obligation, owner, told) in OWNERS {
        assert_eq!(obligation.owner(), owner, "{obligation:?}");
        assert_eq!(obligation.told(), told, "{obligation:?}");
    }
}

// Rule: a wait state added without a row above fails here: the table lists every state the daemon can wait in.
#[test]
fn no_wait_state_is_missing_from_the_table_of_owners() {
    let listed: Vec<Obligation> = OWNERS.iter().map(|(obligation, _, _)| *obligation).collect();
    for obligation in Obligation::iter() {
        assert!(listed.contains(&obligation), "{obligation:?} has no owner in the table");
    }
    assert_eq!(listed.len(), Obligation::iter().count());
}

#[test]
fn the_names_of_the_owners_are_the_texts_of_the_catalog() {
    let cases = [
        (WaitOwner::Lead, "the lead"),
        (WaitOwner::Human, "the human"),
        (WaitOwner::Reviewers, "the reviewers"),
        (WaitOwner::Harness, "the harness"),
    ];
    for (owner, expected) in cases {
        assert_eq!(owner.text(), expected);
    }
}

// Rule: the wait line names who is obliged and since when; an unowned round names nobody, and a wait with no known start
// says no time.
#[test]
fn the_wait_line_names_the_owner_and_since_when() {
    let lead = Waiting::For(Obligation::LeadCorrection);

    assert_eq!(
        lead.line(Some("2026-11-22 03:12")),
        "the lead's correction, owed by the lead since 2026-11-22 03:12"
    );
    assert_eq!(lead.line(None), "the lead's correction, owed by the lead");
    assert_eq!(
        Waiting::For(Obligation::Human).line(Some("03:12")),
        "an answer of the human, owed by the human since 03:12"
    );
    assert_eq!(Waiting::Defect.line(Some("03:12")), "nothing: this is a defect");
}
