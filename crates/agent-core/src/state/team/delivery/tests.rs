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
