//! Delivery transitions preserve owning PR identity and refuse invalid phases atomically.

use super::*;
use crate::state::fixtures::{at, drive, pr, round_id, team};
use crate::state::{Completion, ItemPhase, PhaseKind, RoundEnd};

fn working() -> Team {
    let mut team = team(1);
    for (id, item) in [(1, "1.2"), (2, "2.1")] {
        team.open_round(round_id(id), Some(item.parse().unwrap()), at(1))
            .unwrap();
    }
    drive(&mut team, "1.2", PhaseKind::InReview);
    drive(&mut team, "2.1", PhaseKind::InProgress);
    team
}

#[test]
fn accepted_delivery_waits_then_completes_with_owning_pr_without_review() {
    let mut team = working();
    let item = "2.1".parse().unwrap();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    assert_eq!(team.round(&round_id(2)).unwrap().delivered_by(), Some(&pr(7)));
    assert!(team.round(&round_id(2)).unwrap().is_open());
    assert_eq!(team.item(item).unwrap().phase().kind(), PhaseKind::InProgress);
    let mut team: Team = serde_json::from_str(&serde_json::to_string(&team).unwrap()).unwrap();
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    assert_eq!(
        team.item(item).unwrap().phase(),
        &ItemPhase::Done {
            completion: Completion::Merged { pr: pr(7), at: at(40) }
        }
    );
    assert_eq!(team.round(&round_id(2)).unwrap().ended(), Some(RoundEnd::Merged));
    assert_eq!(team.round(&round_id(2)).unwrap().delivered_by(), Some(&pr(7)));
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap()).unwrap(),
        team
    );
}

#[test]
fn delivery_refusals_leave_team_unchanged() {
    let original = working();
    let mut team = original.clone();
    assert_eq!(
        team.deliver_round(&round_id(3), pr(7).number),
        Err(DeliveryError::NoSuchRound { round: round_id(3) })
    );
    assert_eq!(
        team.deliver_round(&round_id(1), pr(7).number),
        Err(DeliveryError::NotWorking { round: round_id(1) })
    );
    assert_eq!(
        team.deliver_round(&round_id(2), pr(99).number),
        Err(DeliveryError::NoTeamPr { number: pr(99).number })
    );
    assert_eq!(
        team.complete_delivery(&round_id(2), at(40)),
        Err(DeliveryError::NotWaiting { round: round_id(2) })
    );
    assert_eq!(
        team.complete_delivery(&round_id(3), at(40)),
        Err(DeliveryError::NoSuchRound { round: round_id(3) })
    );
    assert_eq!(
        team.complete_delivery(&round_id(1), at(40)),
        Err(DeliveryError::NotWorking { round: round_id(1) })
    );
    assert_eq!(team, original);
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    let waiting = team.clone();
    assert_eq!(
        team.deliver_round(&round_id(2), pr(7).number),
        Err(DeliveryError::AlreadyDelivered { round: round_id(2) })
    );
    assert!(
        matches!(team.complete_delivery(&round_id(2), at(0)), Err(DeliveryError::BeforeOpening(ClosedBeforeOpened { at: found, .. })) if found == at(0))
    );
    assert_eq!(team, waiting);
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    let done = team.clone();
    assert_eq!(
        team.complete_delivery(&round_id(2), at(41)),
        Err(DeliveryError::NotWorking { round: round_id(2) })
    );
    assert_eq!(team, done);
}

#[test]
fn delivery_does_not_repurpose_pending_listed_or_closed_rounds() {
    let mut team = working();
    team.item_mut("2.1".parse().unwrap())
        .unwrap()
        .advance(ItemPhase::Pending)
        .unwrap();
    let pending = team.clone();
    assert_eq!(
        team.deliver_round(&round_id(2), pr(7).number),
        Err(DeliveryError::NotWorking { round: round_id(2) })
    );
    assert_eq!(team, pending);
    team.close_round(&round_id(2), at(40), RoundEnd::Abandoned).unwrap();
    let closed = team.clone();
    assert_eq!(
        team.deliver_round(&round_id(2), pr(7).number),
        Err(DeliveryError::NotWorking { round: round_id(2) })
    );
    assert_eq!(team, closed);
}

#[test]
fn delivery_metadata_cannot_forge_team_ownership_or_item_phase_when_read() {
    let mut team = working();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    let valid = serde_json::to_value(&team).unwrap();
    for (field, replacement) in [
        ("number", serde_json::json!(99)),
        ("branch", serde_json::json!("forged")),
    ] {
        let mut value = valid.clone();
        value["rounds"][1]["delivery"][field] = replacement;
        crate::state::fixtures::assert_refused::<Team>(&value);
    }
    let mut value = valid;
    value["items"][2]["phase"] = serde_json::json!({"phase": "pending"});
    crate::state::fixtures::assert_refused::<Team>(&value);
}

#[test]
fn delivery_refuses_prs_owned_only_by_abandoned_or_approved_rounds() {
    for end in [RoundEnd::Abandoned, RoundEnd::Approved] {
        let mut team = working();
        if end == RoundEnd::Approved {
            team.advance_item(
                "1.2".parse().unwrap(),
                ItemPhase::Approved {
                    pr: pr(7),
                    reviews: Vec::new(),
                },
            )
            .unwrap();
        }
        team.close_round(&round_id(1), at(30), end).unwrap();
        let before = team.clone();
        assert_eq!(
            team.deliver_round(&round_id(2), pr(7).number),
            Err(DeliveryError::NoTeamPr { number: pr(7).number })
        );
        assert_eq!(team, before);
    }
}

#[test]
fn delivery_round_details_show_the_owning_pr_while_the_item_is_working() {
    let mut team = working();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    let detail = crate::protocol::RoundDetail::new(&team, team.round(&round_id(2)).unwrap(), Vec::new());
    assert_eq!(detail.pr, Some(pr(7)));
}
