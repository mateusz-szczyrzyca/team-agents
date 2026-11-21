//! Decisions distinguish missing capability from unavailable qualified crew.

use super::*;
use crate::state::fixtures::{at, inv, member, normal, task, team_id};
use crate::state::{Crew, MergeMode, TaskListText, TeamMembers};
use crate::status::TaskStatus;

#[test]
fn ready_items_use_members_before_the_only_fitting_coordinator_or_wait() {
    for (required, member_tier, coordinator_tier, busy, expected) in [
        ("normal", Tier::Normal, Tier::Investigator, false, "member"),
        ("worker", Tier::Normal, Tier::Investigator, false, "member"),
        ("investigator", Tier::Investigator, Tier::Investigator, false, "member"),
        ("investigator", Tier::Normal, Tier::Investigator, false, "coordinator"),
        ("investigator", Tier::Normal, Tier::Normal, false, "absent"),
        ("normal", Tier::Normal, Tier::Investigator, true, "busy"),
    ] {
        let participant = |tier, name| match tier {
            Tier::Investigator => inv(name),
            Tier::Normal => normal(name),
            Tier::Worker => crate::state::fixtures::worker(name),
        };
        let mut team = Team::new(
            team_id(1),
            "example".parse().unwrap(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(format!("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: {required}\n")).unwrap(),
            Crew::team(TeamMembers {
                coordinator: member(1, participant(coordinator_tier, "lead")),
                members: vec![member(2, participant(member_tier, "member"))],
                workers: vec![],
            })
            .unwrap(),
        );
        if busy {
            team.find_mut(&task(2))
                .unwrap()
                .1
                .transition(TaskStatus::Running)
                .unwrap();
        }
        let item = "1.1".parse().unwrap();
        let chosen = dispatch(&team);
        let wanted = match expected {
            "member" => Dispatch {
                assigned: vec![(item, participant(member_tier, "member"))],
                waiting: vec![],
            },
            "coordinator" => Dispatch {
                assigned: vec![(item, participant(coordinator_tier, "lead"))],
                waiting: vec![],
            },
            "absent" | "busy" => Dispatch {
                assigned: vec![],
                waiting: vec![Waiting {
                    item,
                    required: required.parse().unwrap(),
                    reason: if expected == "absent" {
                        WaitReason::Absent
                    } else {
                        WaitReason::Busy
                    },
                }],
            },
            _ => panic!("known case"),
        };
        assert_eq!(chosen, wanted, "{required}, {member_tier}, {coordinator_tier}, {busy}");
    }
}

#[test]
fn first_split_part_requeues_an_underqualified_parent_assignee() {
    use crate::state::fixtures::{drive, round_id, team};
    use crate::state::{ItemPhase, PhaseKind};
    let item = "1.2".parse().unwrap();
    let mut found = team(1);
    found.open_round(round_id(1), Some(item), at(1)).unwrap();
    drive(&mut found, "1.2", PhaseKind::InProgress);
    let plan = serde_json::from_value(serde_json::json!({
        "parent": "1.2", "parts": [
            {"title":"First", "delivers":"First document", "checks":"Read it", "depends_on":[], "tier":"investigator"},
            {"title":"Second", "delivers":"Second document", "checks":"Read it", "depends_on":["a"]}
        ]
    }))
    .unwrap();
    found.activate_split(plan).unwrap();
    assert_eq!(required_tier(&found, item), Tier::Investigator);
    assert_eq!(underqualified(&found), vec![item]);
    found.advance_item(item, ItemPhase::Pending).unwrap();
    assert_eq!(underqualified(&found), vec![]);
    assert_eq!(dispatch(&found).assigned, vec![(item, inv("inv-opus"))]);
}

#[test]
fn coordinator_takeover_respects_the_work_requirement_rather_than_the_failed_agents_tier() {
    for (required, coordinator_tier, expected) in [
        ("normal", Tier::Normal, true),
        ("normal", Tier::Investigator, true),
        ("investigator", Tier::Normal, false),
        ("investigator", Tier::Investigator, true),
    ] {
        let coordinator = match coordinator_tier {
            Tier::Normal => normal("lead"),
            Tier::Investigator => inv("lead"),
            Tier::Worker => panic!("a worker cannot coordinate"),
        };
        let mut team = Team::new(
            team_id(1),
            "example".parse().unwrap(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(format!("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: {required}\n")).unwrap(),
            Crew::team(TeamMembers {
                coordinator: member(1, coordinator),
                members: vec![member(2, inv("member"))],
                workers: vec![],
            })
            .unwrap(),
        );
        team.open_round(crate::state::fixtures::round_id(1), Some("1.1".parse().unwrap()), at(1))
            .unwrap();
        team.advance_item(
            "1.1".parse().unwrap(),
            crate::state::ItemPhase::InProgress {
                assignee: "member".parse().unwrap(),
                branch: "agents/member/work".parse().unwrap(),
            },
        )
        .unwrap();
        assert_eq!(
            coordinator_can_take_over(&team, team.find(&task(2)).unwrap().1),
            expected,
            "{required}, {coordinator_tier}"
        );
    }
}
