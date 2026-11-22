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

#[test]
fn review_fixes_keep_a_qualified_lead_or_select_a_qualified_member() {
    for (required, member_tier, coordinator_tier, expected) in [
        ("normal", Tier::Normal, Tier::Investigator, Some("lead")),
        ("investigator", Tier::Investigator, Tier::Normal, Some("member")),
        ("investigator", Tier::Normal, Tier::Investigator, Some("lead")),
        ("investigator", Tier::Normal, Tier::Normal, None),
    ] {
        let participant = |tier, name| match tier {
            Tier::Investigator => inv(name),
            Tier::Normal => normal(name),
            Tier::Worker => crate::state::fixtures::worker(name),
        };
        let team = Team::new(
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
        assert_eq!(
            fixer(&team, "1.1".parse().unwrap()).map(|member| member.agent().name()),
            expected,
            "{required}, {member_tier}, {coordinator_tier}"
        );
    }
}

#[test]
fn next_split_part_obeys_previous_member_replacement_and_lead_fallbacks() {
    use crate::state::fixtures::{drive, round_id, run};
    use crate::state::NewRun;
    use crate::state::{ItemPhase, PhaseKind, RoundEnd};

    let item = "1.1".parse().unwrap();
    let plan: crate::state::split::SplitPlan = serde_json::from_value(serde_json::json!({
        "parent": "1.1", "parts": [
            {"title":"First", "delivers":"First document", "checks":"Read it", "depends_on":[], "tier":"normal"},
            {"title":"Second", "delivers":"Second document", "checks":"Read it", "depends_on":["a"], "tier":"normal"}
        ]
    }))
    .unwrap();

    // Case 1: Member who worked on part 1 is free -> gets part 2
    let mut team = Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\n".to_string()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, normal("member-1")), member(3, normal("member-2"))],
            workers: vec![],
        })
        .unwrap(),
    );
    team.open_round(round_id(1), Some(item), at(1)).unwrap();
    drive(&mut team, "1.1", PhaseKind::InProgress);
    team.activate_split(plan.clone()).unwrap();
    // Record run for member-1 on round 1
    team.begin_run(
        &task(2),
        NewRun {
            id: run(1),
            at: at(2),
            session: None,
            rounds: vec![round_id(1)],
        },
    )
    .unwrap();
    // Round 1 merges
    let mut handle = team.item_mut(item).unwrap();
    handle.advance(ItemPhase::InReview { pr: crate::state::fixtures::pr(1), reviews: Vec::new() }).unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle.submit_review(&crate::state::fixtures::task(21), crate::status::ReviewVerdict::Approve, at(40)).unwrap();
    let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else { panic!("in review") };
    handle.advance(ItemPhase::Approved { pr: crate::state::fixtures::pr(1), reviews }).unwrap();
    handle.advance(ItemPhase::Done {
        completion: crate::state::Completion::Merged {
            pr: crate::state::fixtures::pr(1),
            at: at(50),
        },
    }).unwrap();
    team.close_round(&round_id(1), at(50), RoundEnd::Merged).unwrap();
    // After close_round, item is pending for next part
    assert_eq!(team.next_part(item).unwrap().0.to_string(), "1.1b");
    // member-1 is free, so part 2 should go to member-1
    let dispatched = dispatch(&team);
    assert_eq!(dispatched.assigned, vec![(item, normal("member-1"))]);

    // Case 2: member-1 is stopped -> part 2 goes to replacement member-2
    team.find_mut(&task(2)).unwrap().1.transition(TaskStatus::Stopped).unwrap();
    let dispatched = dispatch(&team);
    assert_eq!(dispatched.assigned, vec![(item, normal("member-2"))]);

    // Case 3: member-1 stopped, no other member, lead fits -> part 2 goes to lead
    let mut team_lead = Team::new(
        team_id(2),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\n".to_string()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, normal("member-1"))],
            workers: vec![],
        })
        .unwrap(),
    );
    team_lead.open_round(round_id(2), Some(item), at(1)).unwrap();
    drive(&mut team_lead, "1.1", PhaseKind::InProgress);
    team_lead.activate_split(plan).unwrap();
    team_lead.begin_run(
        &task(2),
        NewRun {
            id: run(2),
            at: at(2),
            session: None,
            rounds: vec![round_id(2)],
        },
    )
    .unwrap();
    let mut handle = team_lead.item_mut(item).unwrap();
    handle.advance(ItemPhase::InReview { pr: crate::state::fixtures::pr(2), reviews: Vec::new() }).unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle.submit_review(&crate::state::fixtures::task(21), crate::status::ReviewVerdict::Approve, at(40)).unwrap();
    let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else { panic!("in review") };
    handle.advance(ItemPhase::Approved { pr: crate::state::fixtures::pr(2), reviews }).unwrap();
    handle.advance(ItemPhase::Done {
        completion: crate::state::Completion::Merged {
            pr: crate::state::fixtures::pr(2),
            at: at(50),
        },
    }).unwrap();
    team_lead.close_round(&round_id(2), at(50), RoundEnd::Merged).unwrap();
    team_lead.find_mut(&task(2)).unwrap().1.transition(TaskStatus::Stopped).unwrap();

    let dispatched = dispatch(&team_lead);
    assert_eq!(dispatched.assigned, vec![(item, normal("lead"))]);
}
