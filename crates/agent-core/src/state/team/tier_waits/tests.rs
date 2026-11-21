//! Capability waits survive serialization without repeated notifications.

use super::*;
use crate::state::fixtures::{at, member, normal, team_id};
use crate::state::{Crew, MergeMode, TaskListText, TeamMembers};

#[test]
fn absent_tier_wait_is_durable_and_reported_once_but_busy_is_not_a_tier_wait() {
    let item = "1.1".parse().unwrap();
    let mut team = Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: investigator\n".into()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, normal("member"))],
            workers: vec![],
        })
        .unwrap(),
    );
    assert_eq!(team.refresh_tier_waits(), vec![(item, Tier::Investigator)]);
    assert_eq!(team.tier_waits(), &BTreeMap::from([(item, Tier::Investigator)]));
    let mut restored: Team = serde_json::from_str(&serde_json::to_string(&team).unwrap()).unwrap();
    assert_eq!(restored, team);
    assert_eq!(restored.refresh_tier_waits(), vec![]);
    let mut old: Team =
        serde_json::from_str(include_str!("../../../../tests/fixtures/team-before-tier-waits.json")).unwrap();
    assert_eq!(old.tier_waits(), &BTreeMap::new());
    assert_eq!(old.refresh_tier_waits(), vec![]);
}

#[test]
fn stored_tier_waits_refuse_items_outside_the_team() {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tests/fixtures/team-before-tier-waits.json")).unwrap();
    value["tier_waits"] = serde_json::json!({"99.1":"investigator"});
    assert!(matches!(serde_json::from_value::<Team>(value), Err(error) if error.is_data()));
}

#[test]
fn an_unstaffed_review_fix_keeps_a_durable_tier_wait_after_restart() {
    use crate::state::fixtures::{drive, review, round_id};
    use crate::state::{PhaseKind, ReviewOutcome};
    use crate::status::ReviewVerdict;
    let item = "1.1".parse().unwrap();
    let mut team = Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: investigator\n".into()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, normal("codex-1"))],
            workers: vec![],
        })
        .unwrap(),
    );
    team.open_round(round_id(1), Some(item), at(1)).unwrap();
    drive(&mut team, "1.1", PhaseKind::InReview);
    team.item_mut(item)
        .unwrap()
        .ask_reviews(vec![review("inv-review", 21, ReviewOutcome::Pending)])
        .unwrap();
    team.item_mut(item)
        .unwrap()
        .submit_review(&crate::state::fixtures::task(21), ReviewVerdict::RequestChanges, at(30))
        .unwrap();
    assert_eq!(team.refresh_tier_waits(), vec![(item, Tier::Investigator)]);
    let mut restored: Team = serde_json::from_str(&serde_json::to_string(&team).unwrap()).unwrap();
    assert_eq!(restored.refresh_tier_waits(), vec![]);
    assert_eq!(restored.tier_waits(), &BTreeMap::from([(item, Tier::Investigator)]));
    assert_eq!(restored.rounds(), team.rounds());
}

#[test]
fn tier_questions_are_once_per_wait_and_survive_reload() {
    let item = "1.1".parse().unwrap();
    let mut team = Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: investigator\n".into()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, normal("member"))],
            workers: vec![],
        })
        .unwrap(),
    );
    assert_eq!(team.unasked_tier_waits(), vec![]);
    team.refresh_tier_waits();
    for asked in [false, true] {
        if asked {
            team.mark_tier_questions();
        }
        let expected = if asked {
            vec![]
        } else {
            vec![(item, Tier::Investigator)]
        };
        assert_eq!(team.unasked_tier_waits(), expected);
        let restored: Team = serde_json::from_str(&serde_json::to_string(&team).unwrap()).unwrap();
        assert_eq!(restored.unasked_tier_waits(), expected);
    }
    let mut old: Team =
        serde_json::from_str(include_str!("../../../../tests/fixtures/team-before-tier-waits.json")).unwrap();
    assert_eq!(old.unasked_tier_waits(), vec![]);
    old.mark_tier_questions();
    assert_eq!(old.unasked_tier_waits(), vec![]);
}

#[test]
fn tier_question_recipient_waits_for_an_idle_lead_and_an_unasked_wait() {
    use crate::state::fixtures::task;
    use crate::status::TaskStatus;
    for status in [
        TaskStatus::Queued,
        TaskStatus::Running,
        TaskStatus::Waiting,
        TaskStatus::Questions,
        TaskStatus::Paused,
        TaskStatus::Stopped,
        TaskStatus::Failed,
        TaskStatus::Done,
    ] {
        for asked in [false, true] {
            let mut lead = member(1, normal("lead"));
            if status != TaskStatus::Queued {
                lead.transition(TaskStatus::Running).unwrap();
                if status != TaskStatus::Running {
                    lead.transition(status).unwrap();
                }
            }
            let mut team = Team::new(
                team_id(1),
                "example".parse().unwrap(),
                at(0),
                MergeMode::Autopilot,
                TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: investigator\n".into()).unwrap(),
                Crew::solo(lead).unwrap(),
            );
            assert_eq!(team.tier_question_task(), None);
            team.refresh_tier_waits();
            if asked {
                team.mark_tier_questions();
            }
            let expected = if !asked && matches!(status, TaskStatus::Queued | TaskStatus::Waiting) {
                Some(task(1))
            } else {
                None
            };
            assert_eq!(team.tier_question_task(), expected, "{status:?}, asked={asked}");
        }
    }
}
