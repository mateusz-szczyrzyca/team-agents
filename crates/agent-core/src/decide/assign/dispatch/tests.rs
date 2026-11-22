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
    use crate::state::NewRun;
    use crate::state::fixtures::{drive, round_id, run};
    use crate::state::{ItemPhase, PhaseKind, RoundEnd};

    let item = "1.1".parse().unwrap();
    let plan: crate::state::split::SplitPlan = serde_json::from_value(serde_json::json!({
        "parent": "1.1", "parts": [
            {"title":"First", "delivers":"First document", "checks":"Read it", "depends_on":[], "tier":"normal"},
            {"title":"Second", "delivers":"Second document", "checks":"Read it", "depends_on":["a"], "tier":"normal"}
        ]
    }))
    .unwrap();

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
    team.activate_split(plan).unwrap();
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
    let mut handle = team.item_mut(item).unwrap();
    handle
        .advance(ItemPhase::InReview {
            pr: crate::state::fixtures::pr(1),
            reviews: Vec::new(),
        })
        .unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle
        .submit_review(
            &crate::state::fixtures::task(21),
            crate::status::ReviewVerdict::Approve,
            at(40),
        )
        .unwrap();
    let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
        panic!("in review")
    };
    handle
        .advance(ItemPhase::Approved {
            pr: crate::state::fixtures::pr(1),
            reviews,
        })
        .unwrap();
    handle
        .advance(ItemPhase::Done {
            completion: crate::state::Completion::Merged {
                pr: crate::state::fixtures::pr(1),
                at: at(50),
            },
        })
        .unwrap();
    team.close_round(&round_id(1), at(50), RoundEnd::Merged).unwrap();

    // member-1 is free, so part 2 should go to member-1
    assert_eq!(team.next_part(item).unwrap().0.to_string(), "1.1b");
    let dispatched = dispatch(&team);
    assert_eq!(dispatched.assigned, vec![(item, normal("member-1"))]);

    // member-1 is stopped -> part 2 goes to replacement member-2
    team.find_mut(&task(2))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();
    let dispatched = dispatch(&team);
    assert_eq!(dispatched.assigned, vec![(item, normal("member-2"))]);
}

#[test]
fn next_split_part_falls_back_to_qualified_lead_when_members_stopped() {
    use crate::state::NewRun;
    use crate::state::fixtures::{drive, round_id, run};
    use crate::state::{ItemPhase, PhaseKind, RoundEnd};

    let item = "1.1".parse().unwrap();
    let plan: crate::state::split::SplitPlan = serde_json::from_value(serde_json::json!({
        "parent": "1.1", "parts": [
            {"title":"First", "delivers":"First document", "checks":"Read it", "depends_on":[], "tier":"normal"},
            {"title":"Second", "delivers":"Second document", "checks":"Read it", "depends_on":["a"], "tier":"normal"}
        ]
    }))
    .unwrap();

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
    team_lead
        .begin_run(
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
    handle
        .advance(ItemPhase::InReview {
            pr: crate::state::fixtures::pr(2),
            reviews: Vec::new(),
        })
        .unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle
        .submit_review(
            &crate::state::fixtures::task(21),
            crate::status::ReviewVerdict::Approve,
            at(40),
        )
        .unwrap();
    let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
        panic!("in review")
    };
    handle
        .advance(ItemPhase::Approved {
            pr: crate::state::fixtures::pr(2),
            reviews,
        })
        .unwrap();
    handle
        .advance(ItemPhase::Done {
            completion: crate::state::Completion::Merged {
                pr: crate::state::fixtures::pr(2),
                at: at(50),
            },
        })
        .unwrap();
    team_lead.close_round(&round_id(2), at(50), RoundEnd::Merged).unwrap();
    team_lead
        .find_mut(&task(2))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();

    let dispatched = dispatch(&team_lead);
    assert_eq!(dispatched.assigned, vec![(item, normal("lead"))]);
}

#[test]
fn split_part_assigns_to_replacement_agent_after_task_handover() {
    use crate::state::NewRun;
    use crate::state::fixtures::{drive, round_id, run};
    use crate::state::{ItemPhase, PhaseKind, RoundEnd};

    let item = "1.1".parse().unwrap();
    let plan: crate::state::split::SplitPlan = serde_json::from_value(serde_json::json!({
        "parent": "1.1", "parts": [
            {"title":"First", "delivers":"First document", "checks":"Read it", "depends_on":[], "tier":"normal"},
            {"title":"Second", "delivers":"Second document", "checks":"Read it", "depends_on":["a"], "tier":"normal"}
        ]
    }))
    .unwrap();

    let mut team = Team::new(
        team_id(3),
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
    team.open_round(round_id(3), Some(item), at(1)).unwrap();
    drive(&mut team, "1.1", PhaseKind::InProgress);
    team.activate_split(plan).unwrap();
    team.begin_run(
        &task(2),
        NewRun {
            id: run(3),
            at: at(2),
            session: None,
            rounds: vec![round_id(3)],
        },
    )
    .unwrap();
    let mut handle = team.item_mut(item).unwrap();
    handle
        .advance(ItemPhase::InReview {
            pr: crate::state::fixtures::pr(3),
            reviews: Vec::new(),
        })
        .unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle
        .submit_review(
            &crate::state::fixtures::task(21),
            crate::status::ReviewVerdict::Approve,
            at(40),
        )
        .unwrap();
    let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
        panic!("in review")
    };
    handle
        .advance(ItemPhase::Approved {
            pr: crate::state::fixtures::pr(3),
            reviews,
        })
        .unwrap();
    handle
        .advance(ItemPhase::Done {
            completion: crate::state::Completion::Merged {
                pr: crate::state::fixtures::pr(3),
                at: at(50),
            },
        })
        .unwrap();
    team.close_round(&round_id(3), at(50), RoundEnd::Merged).unwrap();

    team.find_mut(&task(2))
        .unwrap()
        .1
        .held_task(normal("claude-rep"), at(55));

    assert_eq!(team.next_part(item).unwrap().0.to_string(), "1.1b");
    let dispatched = dispatch(&team);
    assert_eq!(dispatched.assigned, vec![(item, normal("claude-rep"))]);
}

#[test]
fn split_part_prefers_member_over_coordinator_when_both_participated_in_previous_round() {
    use crate::state::fixtures::{drive, round_id, run, team};
    use crate::state::{NewRun, PhaseKind, RoundEnd};
    let item = "1.2".parse().unwrap();
    let mut team = team(1);
    let plan = serde_json::from_value(serde_json::json!({
        "parent": "1.2", "parts": [
            {"title":"First", "delivers":"First document", "checks":"Read it", "depends_on":[]},
            {"title":"Second", "delivers":"Second document", "checks":"Read it", "depends_on":["a"]}
        ]
    }))
    .unwrap();
    team.open_round(round_id(1), Some(item), at(1)).unwrap();
    drive(&mut team, "1.2", PhaseKind::InProgress);
    team.activate_split(plan).unwrap();
    // Coordinator (task 1) runs in round 1
    team.begin_run(
        &task(1),
        NewRun {
            id: run(1),
            at: at(2),
            session: None,
            rounds: vec![round_id(1)],
        },
    )
    .unwrap();
    // Member (task 2) also runs in round 1
    team.begin_run(
        &task(2),
        NewRun {
            id: run(2),
            at: at(3),
            session: None,
            rounds: vec![round_id(1)],
        },
    )
    .unwrap();
    let mut handle = team.item_mut(item).unwrap();
    handle
        .advance(crate::state::ItemPhase::InReview {
            pr: crate::state::fixtures::pr(1),
            reviews: Vec::new(),
        })
        .unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle
        .submit_review(
            &crate::state::fixtures::task(21),
            crate::status::ReviewVerdict::Approve,
            at(5),
        )
        .unwrap();
    let crate::state::ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
        panic!("in review")
    };
    handle
        .advance(crate::state::ItemPhase::Approved {
            pr: crate::state::fixtures::pr(1),
            reviews,
        })
        .unwrap();
    handle
        .advance(crate::state::ItemPhase::Done {
            completion: crate::state::Completion::Merged {
                pr: crate::state::fixtures::pr(1),
                at: at(10),
            },
        })
        .unwrap();
    team.close_round(&round_id(1), at(10), RoundEnd::Merged).unwrap();
    assert_eq!(team.next_part(item).unwrap().0.to_string(), "1.2b");
    // Under ReplacementMode::Off, replacement is disabled: only the previous member gets the part
    let dispatched = dispatch_in_mode(&team, crate::decide::replacement::ReplacementMode::Off);
    assert_eq!(dispatched.assigned, vec![(item, normal("codex-1"))]);
}

#[test]
fn stopped_member_under_off_mode_waits_absent_and_enters_tier_waits() {
    use crate::state::fixtures::{at, drive, inv, member, normal, round_id, task, team_id};
    use crate::state::{Crew, ItemPhase, MergeMode, PhaseKind, RoundEnd, TaskListText, TeamMembers};

    let item = "1.1".parse().unwrap();
    let mut team = Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: normal\n".to_owned()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, inv("inv-lead")),
            members: vec![member(2, normal("claude-1"))],
            workers: vec![],
        })
        .unwrap(),
    );
    team.open_round(round_id(1), Some(item), at(1)).unwrap();
    drive(&mut team, "1.1", PhaseKind::InProgress);
    let plan = serde_json::from_value(serde_json::json!({
        "parent": "1.1",
        "parts": [
            {"title": "A", "delivers": "A", "checks": "A", "depends_on": []},
            {"title": "B", "delivers": "B", "checks": "B", "depends_on": ["a"]}
        ]
    }))
    .unwrap();
    team.activate_split(plan).unwrap();
    let mut handle = team.item_mut(item).unwrap();
    handle
        .advance(crate::state::ItemPhase::InReview {
            pr: crate::state::fixtures::pr(1),
            reviews: Vec::new(),
        })
        .unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle
        .submit_review(
            &crate::state::fixtures::task(21),
            crate::status::ReviewVerdict::Approve,
            at(5),
        )
        .unwrap();
    let crate::state::ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
        panic!("in review")
    };
    handle
        .advance(crate::state::ItemPhase::Approved {
            pr: crate::state::fixtures::pr(1),
            reviews,
        })
        .unwrap();
    handle
        .advance(ItemPhase::Done {
            completion: crate::state::Completion::Merged {
                pr: crate::state::fixtures::pr(1),
                at: at(10),
            },
        })
        .unwrap();
    team.close_round(&round_id(1), at(10), RoundEnd::Merged).unwrap();

    // Now claude-1 stops:
    team.find_mut(&task(2))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();

    // Under ReplacementMode::Off, replacement is disabled, and lead cannot take it:
    // It must return WaitReason::Absent, NEVER WaitReason::Busy:
    let dispatched = dispatch_in_mode(&team, crate::decide::replacement::ReplacementMode::Off);
    assert_eq!(dispatched.assigned, vec![]);
    assert_eq!(
        dispatched.waiting,
        vec![Waiting {
            item,
            required: Tier::Normal,
            reason: WaitReason::Absent,
        }]
    );
}

#[test]
fn configured_replacement_ordering_and_exclusion_is_respected() {
    use crate::state::fixtures::{at, drive, inv, member, normal, round_id, task, team_id};
    use crate::state::{Crew, ItemPhase, MergeMode, PhaseKind, RoundEnd, TaskListText, TeamMembers};

    let item = "1.1".parse().unwrap();
    let mut team = Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: normal\n".to_owned()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, inv("inv-lead")),
            members: vec![
                member(2, normal("claude-1")),
                member(3, normal("claude-2")),
                member(4, normal("claude-3")),
            ],
            workers: vec![],
        })
        .unwrap(),
    );
    team.open_round(round_id(1), Some(item), at(1)).unwrap();
    drive(&mut team, "1.1", PhaseKind::InProgress);
    let plan = serde_json::from_value(serde_json::json!({
        "parent": "1.1",
        "parts": [
            {"title": "A", "delivers": "A", "checks": "A", "depends_on": []},
            {"title": "B", "delivers": "B", "checks": "B", "depends_on": ["a"]}
        ]
    }))
    .unwrap();
    team.activate_split(plan).unwrap();
    let mut handle = team.item_mut(item).unwrap();
    handle
        .advance(crate::state::ItemPhase::InReview {
            pr: crate::state::fixtures::pr(1),
            reviews: Vec::new(),
        })
        .unwrap();
    let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
    handle.ask_reviews(vec![reviewer]).unwrap();
    handle
        .submit_review(
            &crate::state::fixtures::task(21),
            crate::status::ReviewVerdict::Approve,
            at(5),
        )
        .unwrap();
    let crate::state::ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
        panic!("in review")
    };
    handle
        .advance(crate::state::ItemPhase::Approved {
            pr: crate::state::fixtures::pr(1),
            reviews,
        })
        .unwrap();
    handle
        .advance(ItemPhase::Done {
            completion: crate::state::Completion::Merged {
                pr: crate::state::fixtures::pr(1),
                at: at(10),
            },
        })
        .unwrap();
    team.close_round(&round_id(1), at(10), RoundEnd::Merged).unwrap();

    // Now claude-1 stops:
    team.find_mut(&task(2))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();

    // With configured replacement ["claude-3", "claude-2"], claude-3 is chosen over claude-2:
    let rep_3: AgentName = "claude-3".parse().unwrap();
    let rep_2: AgentName = "claude-2".parse().unwrap();
    let dispatched = dispatch_in_policy(
        &team,
        crate::decide::replacement::ReplacementMode::Auto,
        &[rep_3.clone(), rep_2.clone()],
    );
    assert_eq!(dispatched.assigned, vec![(item, normal("claude-3"))]);

    // With configured replacement ["claude-2"], claude-3 is excluded:
    let dispatched_2 = dispatch_in_policy(
        &team,
        crate::decide::replacement::ReplacementMode::Auto,
        &[rep_2],
    );
    assert_eq!(dispatched_2.assigned, vec![(item, normal("claude-2"))]);

    // With configured replacement ["other"], crew members claude-2 and claude-3 are excluded;
    // falls back to coordinator inv-lead:
    let rep_other: AgentName = "other".parse().unwrap();
    let dispatched_lead = dispatch_in_policy(
        &team,
        crate::decide::replacement::ReplacementMode::Auto,
        &[rep_other],
    );
    assert_eq!(dispatched_lead.assigned, vec![(item, inv("inv-lead"))]);
}
