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
        Err(DeliverRoundError::NoSuchRound { round: round_id(3) })
    );
    assert_eq!(
        team.deliver_round(&round_id(1), pr(7).number),
        Err(DeliverRoundError::NotWorking { round: round_id(1) })
    );
    assert_eq!(
        team.deliver_round(&round_id(2), pr(99).number),
        Err(DeliverRoundError::NoTeamPr { number: pr(99).number })
    );
    assert_eq!(
        team.complete_delivery(&round_id(2), at(40)),
        Err(CompleteDeliveryError::NotWaiting { round: round_id(2) })
    );
    assert_eq!(
        team.complete_delivery(&round_id(3), at(40)),
        Err(CompleteDeliveryError::NoSuchRound { round: round_id(3) })
    );
    assert_eq!(
        team.complete_delivery(&round_id(1), at(40)),
        Err(CompleteDeliveryError::NotWorking { round: round_id(1) })
    );
    assert_eq!(team, original);
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    let waiting = team.clone();
    assert_eq!(
        team.deliver_round(&round_id(2), pr(7).number),
        Err(DeliverRoundError::AlreadyDelivered { round: round_id(2) })
    );
    assert!(
        matches!(team.complete_delivery(&round_id(2), at(0)), Err(CompleteDeliveryError::BeforeOpening(ClosedBeforeOpened { at: found, .. })) if found == at(0))
    );
    assert_eq!(team, waiting);
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    let done = team.clone();
    assert_eq!(
        team.complete_delivery(&round_id(2), at(41)),
        Err(CompleteDeliveryError::NotWorking { round: round_id(2) })
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
        Err(DeliverRoundError::NotWorking { round: round_id(2) })
    );
    assert_eq!(team, pending);
    team.close_round(&round_id(2), at(40), RoundEnd::Abandoned).unwrap();
    let closed = team.clone();
    assert_eq!(
        team.deliver_round(&round_id(2), pr(7).number),
        Err(DeliverRoundError::NotWorking { round: round_id(2) })
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
fn delivery_retains_team_pr_ownership_after_its_round_closes() {
    for end in [RoundEnd::Abandoned, RoundEnd::Approved] {
        let mut team = working();
        if end == RoundEnd::Approved {
            let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
            item.ask_reviews(vec![crate::state::fixtures::review(
                "inv-codex",
                6,
                crate::state::ReviewOutcome::Pending,
            )])
            .unwrap();
            item.submit_review(
                &crate::state::fixtures::task(6),
                crate::status::ReviewVerdict::Approve,
                at(25),
            )
            .unwrap();
            let ItemPhase::InReview { pr, reviews } = item.phase().clone() else {
                panic!("in review")
            };
            item.advance(ItemPhase::Approved { pr, reviews }).unwrap();
        }
        team.close_round(&round_id(1), at(30), end).unwrap();
        assert_eq!(team.deliver_round(&round_id(2), pr(7).number), Ok(()));
        assert_eq!(team.round(&round_id(2)).unwrap().delivered_by(), Some(&pr(7)));
    }
}

#[test]
fn delivery_round_details_show_the_owning_pr_while_the_item_is_working() {
    let mut team = working();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    let detail = crate::protocol::RoundDetail::new(&team, team.round(&round_id(2)).unwrap(), Vec::new());
    assert_eq!(detail.pr, Some(pr(7)));
}

#[test]
fn split_delivery_keeps_parent_pending_until_its_last_delivered_part() {
    let mut team = working();
    let parent = "2.1".parse().unwrap();
    let plan = crate::state::split::SplitPlan::new(
        parent,
        ["Résumé draft", "Publish résumé"]
            .into_iter()
            .map(|title| crate::state::split::PartDefinition {
                title: title.into(),
                delivers: "A useful document".into(),
                checks: "Read the document".into(),
                depends_on: Vec::new(),
            })
            .collect(),
    )
    .unwrap();
    team.activate_split(plan).unwrap();
    for (number, at_end, expected) in [(2, 40, PhaseKind::Pending), (3, 80, PhaseKind::Done)] {
        if number == 3 {
            team.open_round(round_id(number), Some(parent), at(60)).unwrap();
            drive(&mut team, "2.1", PhaseKind::InProgress);
        }
        team.deliver_round(&round_id(number), pr(7).number).unwrap();
        team.complete_delivery(&round_id(number), at(at_end)).unwrap();
        let round = team.round(&round_id(number)).unwrap();
        assert_eq!(round.part_pull_request(), Some(&pr(7)));
        assert_eq!(round.delivered_by(), Some(&pr(7)));
        assert_eq!(team.item(parent).unwrap().phase().kind(), expected);
        assert_eq!(
            serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap()).unwrap(),
            team
        );
    }
}

fn waiting_ids(team: &Team) -> Vec<RoundId> {
    team.delivery_waits().map(|round| round.id().clone()).collect()
}

#[test]
fn an_accepted_delivery_waits_while_its_round_is_open_and_after_a_stop_abandoned_it() {
    let mut team = working();
    assert_eq!(waiting_ids(&team), Vec::<RoundId>::new());
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    assert_eq!(waiting_ids(&team), vec![round_id(2)]);
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    assert_eq!(waiting_ids(&team), vec![round_id(2)]);
}

#[test]
fn an_abandoned_delivery_completes_with_its_pr_and_keeps_its_round_closed_as_abandoned() {
    let mut team = working();
    let item = "2.1".parse().unwrap();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    assert_eq!(
        team.item(item).unwrap().phase(),
        &ItemPhase::Done {
            completion: Completion::Merged { pr: pr(7), at: at(40) }
        }
    );
    let round = team.round(&round_id(2)).unwrap();
    assert_eq!(round.ended(), Some(RoundEnd::Abandoned));
    assert_eq!(round.delivered_by(), Some(&pr(7)));
    assert_eq!(waiting_ids(&team), Vec::<RoundId>::new());
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap()).unwrap(),
        team
    );
}

#[test]
fn an_abandoned_round_without_a_claim_cannot_complete_a_delivery() {
    let mut team = working();
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    let abandoned = team.clone();
    assert_eq!(
        team.complete_delivery(&round_id(2), at(40)),
        Err(CompleteDeliveryError::NotWaiting { round: round_id(2) })
    );
    assert_eq!(team, abandoned);
}

/// A team whose item 2.1 is split in two parts without dependencies; round 2 is the round of part a.
fn two_part_team() -> Team {
    let mut team = working();
    let plan = crate::state::split::SplitPlan::new(
        "2.1".parse().unwrap(),
        ["First part", "Second part"]
            .into_iter()
            .map(|title| crate::state::split::PartDefinition {
                title: title.into(),
                delivers: "A useful document".into(),
                checks: "Read the document".into(),
                depends_on: Vec::new(),
            })
            .collect(),
    )
    .unwrap();
    team.activate_split(plan).unwrap();
    team
}

#[test]
fn a_completed_part_delivery_does_not_wait_again_when_the_next_part_starts() {
    let mut team = two_part_team();
    let parent = "2.1".parse().unwrap();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    team.open_round(round_id(3), Some(parent), at(60)).unwrap();
    drive(&mut team, "2.1", PhaseKind::InProgress);
    assert_eq!(waiting_ids(&team), Vec::<RoundId>::new());
    team.deliver_round(&round_id(3), pr(7).number).unwrap();
    assert_eq!(waiting_ids(&team), vec![round_id(3)]);
}

#[test]
fn an_abandoned_part_delivery_completes_into_a_state_that_reads_back() {
    let mut team = two_part_team();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    let round = team.round(&round_id(2)).unwrap();
    assert_eq!(round.part_pull_request(), Some(&pr(7)));
    assert_eq!(round.ended(), Some(RoundEnd::Abandoned));
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap()).unwrap(),
        team
    );
}

#[test]
fn an_abandoned_delivery_cannot_complete_before_its_round_opened() {
    let mut team = working();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    let abandoned = team.clone();
    assert!(matches!(
        team.complete_delivery(&round_id(2), at(0)),
        Err(CompleteDeliveryError::BeforeOpening(ClosedBeforeOpened { at: found, .. })) if found == at(0)
    ));
    assert_eq!(team, abandoned);
}

#[test]
fn a_closed_claim_returns_ordinary_and_split_items_to_a_fresh_round() {
    for mut team in [working(), two_part_team()] {
        let item = "2.1".parse().unwrap();
        team.deliver_round(&round_id(2), pr(7).number).unwrap();
        let part = team.round(&round_id(2)).unwrap().part();
        let phase = team.item(item).unwrap().phase().clone();
        team.end_delivery(&round_id(2), at(40)).unwrap();
        let old = team.round(&round_id(2)).unwrap();
        assert_eq!(old.delivery_closed_at(), Some(at(40)));
        assert_eq!(old.ended(), Some(RoundEnd::Abandoned));
        assert_eq!(old.delivered_by(), Some(&pr(7)));
        assert_eq!(waiting_ids(&team), Vec::<RoundId>::new());
        let mut team: Team = serde_json::from_str(&serde_json::to_string(&team).unwrap()).unwrap();
        assert_eq!(team.open_round(round_id(3), Some(item), at(41)), Ok(()));
        assert_eq!(team.round_of(item).unwrap().part(), part);
        assert_eq!(team.round_of(item).unwrap().delivered_by(), None);
        assert_eq!(team.item(item).unwrap().phase(), &phase);
        assert_eq!(
            serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap()).unwrap(),
            team
        );
        assert_eq!(
            team.complete_delivery(&round_id(2), at(42)),
            Err(CompleteDeliveryError::NotWaiting { round: round_id(2) })
        );
    }
}

#[test]
fn ending_a_delivery_refuses_each_invalid_transition_without_mutation() {
    let mut team = working();
    for (id, error) in [
        (round_id(9), EndDeliveryError::NoSuchRound { round: round_id(9) }),
        (round_id(1), EndDeliveryError::NotWorking { round: round_id(1) }),
        (round_id(2), EndDeliveryError::NotWaiting { round: round_id(2) }),
    ] {
        let before = team.clone();
        assert_eq!(team.end_delivery(&id, at(40)), Err(error));
        assert_eq!(team, before);
    }
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    let before = team.clone();
    assert!(
        matches!(team.end_delivery(&round_id(2), at(0)), Err(EndDeliveryError::BeforeOpening(ClosedBeforeOpened { at: found, .. })) if found == at(0))
    );
    assert_eq!(team, before);
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    team.end_delivery(&round_id(2), at(40)).unwrap();
    assert_eq!(team.round(&round_id(2)).unwrap().delivery_closed_at(), Some(at(40)));
    let before = team.clone();
    assert_eq!(
        team.end_delivery(&round_id(2), at(41)),
        Err(EndDeliveryError::NotWaiting { round: round_id(2) })
    );
    assert_eq!(team, before);
}

#[test]
fn persisted_ended_delivery_requires_a_claim_abandoned_round_and_valid_time() {
    let mut team = working();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    team.end_delivery(&round_id(2), at(40)).unwrap();
    let valid = serde_json::to_value(&team).unwrap();
    for (field, value) in [
        ("delivery", serde_json::Value::Null),
        ("state", serde_json::json!({"status":"open"})),
        ("delivery_closed_at", serde_json::to_value(at(0)).unwrap()),
    ] {
        let mut invalid = valid.clone();
        invalid["rounds"][1][field] = value;
        let result = serde_json::from_value::<Team>(invalid);
        assert_eq!(result.expect_err(field).classify(), serde_json::error::Category::Data);
    }
}

#[test]
fn ending_a_completed_abandoned_part_refuses_a_stale_claim_without_mutation() {
    let mut team = two_part_team();
    team.deliver_round(&round_id(2), pr(7).number).unwrap();
    team.close_round(&round_id(2), at(30), RoundEnd::Abandoned).unwrap();
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    team.open_round(round_id(3), Some("2.1".parse().unwrap()), at(60))
        .unwrap();
    drive(&mut team, "2.1", PhaseKind::InProgress);
    let before = team.clone();
    for observation in [
        crate::decide::delivery::DeliveryObservation::Open,
        crate::decide::delivery::DeliveryObservation::Closed,
        crate::decide::delivery::DeliveryObservation::Merged,
    ] {
        assert_eq!(
            crate::decide::delivery::delivery_action(&team, &round_id(2), observation),
            crate::decide::delivery::DeliveryAction::Ignore
        );
    }
    assert_eq!(
        team.end_delivery(&round_id(2), at(70)),
        Err(EndDeliveryError::NotWaiting { round: round_id(2) })
    );
    assert_eq!(team, before);
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap()).unwrap(),
        before
    );
}
