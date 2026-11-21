//! First and later approved scopes have stable deliverable and dependency context.

use super::*;
use crate::fixtures::{at, pr};
use agent_core::state::ItemPhase;
use agent_core::state::split::{PartDefinition, SplitPlan};

#[tokio::test]
async fn first_and_later_part_prompts_match_their_scopes() {
    let texts = texts().await;
    let mut team = crew_of(CHAIN, 1, 0);
    let parent = "1.1".parse().unwrap();
    let work = branch("agents/claude-1/first");
    team.open_round("round-20261111-153001-r7q2".parse().unwrap(), Some(parent), at(1))
        .unwrap();
    team.advance_item(
        parent,
        ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: work.clone(),
        },
    )
    .unwrap();
    let part = |title: &str, dependencies| PartDefinition {
        title: title.into(),
        delivers: "An independent résumé".into(),
        checks: "Read the document".into(),
        depends_on: dependencies,
    };
    team.activate_split(
        SplitPlan::new(
            parent,
            vec![
                part("First", vec![]),
                part("Second", vec!["a".to_owned().try_into().unwrap()]),
            ],
        )
        .unwrap(),
    )
    .unwrap();
    let base = branch("main");
    let run = TeamRun {
        team: &team,
        role: Role::Coordinator,
        branch: &work,
        base: &base,
        why: None,
        new_session: false,
    };
    assert_eq!(
        with_part(&texts.team, &run, "CONTEXT".into()).unwrap(),
        include_str!("split-first.txt")
    );

    let mut wire = serde_json::to_value(&team).unwrap();
    wire["rounds"][0]["state"] = serde_json::json!({"status":"closed","at":at(2),"how":"merged"});
    wire["rounds"][0]["part"] = serde_json::json!({"status":"merged","key":"a","pr":pr(1)});
    let mut second = wire["rounds"][0].clone();
    second["id"] = "round-20261111-153002-r7q2".into();
    second["state"] = serde_json::json!({"status":"open"});
    second["part"] = serde_json::json!({"status":"working","key":"b"});
    wire["rounds"].as_array_mut().unwrap().push(second);
    let later: Team = serde_json::from_value(wire).unwrap();
    let run = TeamRun {
        team: &later,
        role: Role::Member,
        branch: &work,
        base: &base,
        why: None,
        new_session: false,
    };
    assert_eq!(
        with_part(&texts.team, &run, "CONTEXT".into()).unwrap(),
        include_str!("split-later.txt")
    );
}

#[tokio::test]
async fn a_coordinator_without_the_part_branch_is_not_told_to_deliver_it() {
    let texts = texts().await;
    let mut team = crew_of(CHAIN, 1, 0);
    let parent = "1.1".parse().unwrap();
    team.open_round("round-20261111-153001-r7q2".parse().unwrap(), Some(parent), at(1))
        .unwrap();
    team.advance_item(
        parent,
        ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: branch("agents/claude-1/first"),
        },
    )
    .unwrap();
    team.activate_split(
        SplitPlan::new(
            parent,
            vec![
                PartDefinition {
                    title: "First".into(),
                    delivers: "First document".into(),
                    checks: "Read first".into(),
                    depends_on: vec![],
                },
                PartDefinition {
                    title: "Second".into(),
                    delivers: "Second document".into(),
                    checks: "Read second".into(),
                    depends_on: vec!["a".to_owned().try_into().unwrap()],
                },
            ],
        )
        .unwrap(),
    )
    .unwrap();
    let base = branch("main");
    let own = branch("agents/inv-opus/planning");
    let run = TeamRun {
        team: &team,
        role: Role::Coordinator,
        branch: &own,
        base: &base,
        why: None,
        new_session: false,
    };

    assert_eq!(with_part(&texts.team, &run, "CONTEXT".into()).unwrap(), "CONTEXT");
}
