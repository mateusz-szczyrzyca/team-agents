use super::*;
use crate::state::fixtures::{at, drive, item, pr, round_id, team, team_id, team_of};
use crate::state::{
    Completion, ItemPhase, PhaseKind, RoundEnd,
    split::{PartDefinition, SplitPlan},
};

#[test]
fn recovery_keeps_merged_items_and_resets_unmerged_work() {
    let mut old = team(1);
    old.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
    drive(&mut old, "1.2", PhaseKind::Done);
    old.close_round(&round_id(1), at(60), RoundEnd::Merged).unwrap();
    old.open_round(round_id(2), Some(item("2.1")), at(70)).unwrap();
    drive(&mut old, "2.1", PhaseKind::InProgress);
    let crew = team_of(2, 10).crew().clone();
    let fresh = old.recovery_team(team_id(2), at(100), crew.clone()).unwrap();
    assert_eq!(
        fresh.item(item("1.1")).unwrap().phase(),
        &ItemPhase::Done {
            completion: Completion::Listed
        }
    );
    assert_eq!(fresh.item(item("1.2")), old.item(item("1.2")));
    assert_eq!(fresh.item(item("2.1")).unwrap().phase(), &ItemPhase::Pending);
    assert_eq!(fresh.rounds(), &old.rounds()[..1]);
    assert_eq!(fresh.crew(), &crew);
    assert_eq!(fresh.repo(), old.repo());
    assert_eq!(fresh.task_list(), old.task_list());
    assert_eq!(fresh.merge(), old.merge());
    assert_eq!(fresh.id(), &team_id(2));
    assert_eq!(fresh.created_at(), at(100));
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&fresh).unwrap()).unwrap(),
        fresh
    );
}

#[test]
fn recovery_keeps_merged_split_parts_without_a_round_for_the_next_part() {
    let mut old = team(1);
    old.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
    drive(&mut old, "1.2", PhaseKind::InProgress);
    old.activate_split(
        SplitPlan::new(
            item("1.2"),
            vec![
                PartDefinition {
                    title: "Résumé".into(),
                    delivers: "Document".into(),
                    checks: "Read it".into(),
                    depends_on: vec![],
                },
                PartDefinition {
                    title: "Illustration".into(),
                    delivers: "Drawing".into(),
                    checks: "View it".into(),
                    depends_on: vec!["a".to_owned().try_into().unwrap()],
                },
            ],
        )
        .unwrap(),
    )
    .unwrap();
    old.deliver_round_with_pr(&round_id(1), pr(7)).unwrap();
    old.complete_delivery(&round_id(1), at(60)).unwrap();
    let fresh = old
        .recovery_team(team_id(2), at(100), team_of(2, 10).crew().clone())
        .unwrap();
    assert_eq!(fresh.splits(), old.splits());
    assert_eq!(fresh.rounds(), old.rounds());
    assert_eq!(fresh.next_part(item("1.2")).unwrap().0.to_string(), "1.2b");
    assert_eq!(fresh.open_rounds().count(), 0);
    assert_eq!(fresh.item(item("1.2")).unwrap().phase(), &ItemPhase::Pending);
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&fresh).unwrap()).unwrap(),
        fresh
    );
}

#[test]
fn recovery_starts_fresh_sessions_for_solo_and_team_crews() {
    use crate::state::fixtures::{member, normal, run, task};
    for mut crew in [
        Crew::solo(member(10, normal("writer"))).unwrap(),
        team_of(2, 10).crew().clone(),
    ] {
        let expected = crew.clone();
        crew.find_mut(&task(10))
            .unwrap()
            .1
            .begin_run(run(1), at(10), None)
            .unwrap();
        let fresh = team(1).recovery_team(team_id(2), at(100), crew).unwrap();
        assert_eq!(fresh.crew(), &expected);
    }
}

#[test]
fn recovery_retains_merge_evidence_before_the_old_round_is_closed() {
    for ending in [None, Some(RoundEnd::Approved), Some(RoundEnd::Merged)] {
        let mut old = team(1);
        old.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
        drive(&mut old, "1.2", PhaseKind::Done);
        if let Some(ending) = ending {
            old.close_round(&round_id(1), at(60), ending).unwrap();
        }
        let recovered = old.recovery_team(team_id(2), at(100), team_of(2, 10).crew().clone());
        assert_eq!(
            recovered.map(|fresh| (
                fresh.item(item("1.2")).cloned(),
                fresh.rounds().len(),
                fresh.rounds().first().map(|round| (round.id().clone(), round.ended())),
                fresh.open_rounds().count(),
            )),
            Ok((
                old.item(item("1.2")).cloned(),
                1,
                Some((round_id(1), Some(ending.unwrap_or(RoundEnd::Merged)))),
                0
            ))
        );
    }
}
