//! Bus membership follows a task before the replacement can run.

use std::collections::{BTreeMap, BTreeSet};

use agent_core::decide::ladder::Trouble;
use agent_core::kinds::Role;
use agent_runtime::agentbus::{BusTier, MemberState, NewMember, NewTeam};

use super::super::fixture::{Daemon, agentbus_binary};
use crate::fixtures::{at, repo, task, team_id};

async fn create_bus_team(owner: &super::super::Owner) {
    owner
        .parts
        .bus
        .team_create(&NewTeam {
            id: team_id(0),
            repo: repo(),
            lead: None,
            members: BTreeMap::from([(
                "claude-1".parse().unwrap(),
                NewMember {
                    task: task(2),
                    branch: "agents/claude-1/1.1".parse().unwrap(),
                    worktree: owner
                        .parts
                        .store
                        .home()
                        .worktree_dir(&repo(), &task(2))
                        .to_str()
                        .unwrap()
                        .parse()
                        .unwrap(),
                    tier: BusTier::Normal,
                    state: MemberState::Sleeping,
                    provider: Some("claude".to_owned()),
                },
            )]),
        })
        .await
        .unwrap();
}

fn block_launches(owner: &mut super::super::Owner) {
    owner.parts.free_space = agent_runtime::disk::SharedFreeSpace::new(agent_runtime::disk::FakeFreeSpace::new(0));
}

#[tokio::test]
async fn scheduler_replaces_the_member_on_the_bus_and_persists_completion() {
    let daemon = Daemon::new(&crate::fixtures::team_state(), None);
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    create_bus_team(&owner).await;
    owner.take_over(&task(2), Role::Member, Trouble::RoundOverdue).unwrap();
    owner.save().await.unwrap();
    block_launches(&mut owner);
    owner.start_queued().await.unwrap();
    let bus = owner.parts.bus.team_get(&team_id(0)).await.unwrap();
    assert_eq!(
        bus.members.keys().map(ToString::to_string).collect::<Vec<_>>(),
        ["claude-2"]
    );
    assert_eq!(owner.parts.state.pending_team_replaces(), &BTreeSet::new());
    let saved =
        agent_core::state::State::from_json(&std::fs::read_to_string(daemon.home.state_file()).unwrap()).unwrap();
    assert_eq!(saved.pending_team_replaces(), &BTreeSet::new());
}

#[tokio::test]
async fn busy_handover_records_the_same_bus_operation() {
    let mut state = crate::fixtures::team_state();
    state.start_waiting(&task(2), at(0)).unwrap();
    let daemon = Daemon::new(&state, None);
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    create_bus_team(&owner).await;
    assert!(owner.hand_over_busy_wait(&task(2)).await.unwrap());
    assert_eq!(
        owner.parts.state.pending_team_replaces(),
        &BTreeSet::from([agent_core::state::PendingTeamReplace {
            team: team_id(0),
            from: "claude-1".parse().unwrap(),
            to: "claude-2".parse().unwrap(),
        }])
    );
    assert!(owner.try_pending_team_replace_for(&task(2)).await.unwrap());
    let bus = owner.parts.bus.team_get(&team_id(0)).await.unwrap();
    assert_eq!(
        bus.members.keys().map(ToString::to_string).collect::<Vec<_>>(),
        ["claude-2"]
    );
}

#[tokio::test]
async fn solo_handover_has_no_bus_operation() {
    let daemon = Daemon::new(&crate::fixtures::solo_state(), None);
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.take_over(&task(1), Role::Solo, Trouble::RoundOverdue).unwrap();
    assert_eq!(owner.parts.state.pending_team_replaces(), &BTreeSet::new());
}

fn flaky_daemon() -> Daemon {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flaky-agentbus.sh");
    let daemon = Daemon::new(&crate::fixtures::team_state(), Some(&script));
    std::fs::write(
        daemon.home.root().join("real-agentbus"),
        agentbus_binary().to_str().unwrap(),
    )
    .unwrap();
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\n");
    daemon
}

#[tokio::test]
async fn storage_failure_keeps_pending_operation_for_the_next_scheduler_pass() {
    let daemon = flaky_daemon();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    create_bus_team(&owner).await;
    owner.take_over(&task(2), Role::Member, Trouble::RoundOverdue).unwrap();
    owner.save().await.unwrap();
    block_launches(&mut owner);
    let failure = daemon.home.root().join("flaky-bus-down");
    std::fs::write(&failure, "").unwrap();
    owner.start_queued().await.unwrap();
    assert!(
        daemon.home.root().join("flaky-bus-refused").exists(),
        "the scheduler must attempt team-replace"
    );
    assert_eq!(
        std::fs::read_to_string(daemon.home.root().join("flaky-bus-refused")).unwrap(),
        "team-replace\n"
    );
    assert_eq!(owner.parts.state.pending_team_replaces().len(), 1);
    std::fs::remove_file(failure).unwrap();
    owner.start_queued().await.unwrap();
    assert_eq!(owner.parts.state.pending_team_replaces(), &BTreeSet::new());
    let bus = owner.parts.bus.team_get(&team_id(0)).await.unwrap();
    assert_eq!(
        bus.members.keys().map(ToString::to_string).collect::<Vec<_>>(),
        ["claude-2"]
    );
}

#[tokio::test]
async fn not_found_settles_the_pending_operation() {
    use super::rig::{ONE, Rig, investigator, normal};
    use agent_core::state::MergeMode;
    use agent_core::status::TaskStatus;
    use agent_runtime::agentbus::AgentBus;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            normal("claude-2", "hold"),
            investigator("inv-lead", "sleep"),
            investigator("inv-review-1", "sleep"),
            investigator("inv-review-2", "sleep"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, Some("inv-lead"))
        .await;
    let state = rig
        .state_where("the original member starts", |state| {
            tasks.iter().any(|task| {
                state.find_task(task).is_some_and(|found| {
                    found.member.agent().name() == "claude-1" && found.member.status() == TaskStatus::Running
                })
            })
        })
        .await;
    let task = tasks
        .iter()
        .find(|task| state.find_task(task).unwrap().member.agent().name() == "claude-1")
        .unwrap()
        .clone();
    rig.stop().await.unwrap();
    let mut state = rig.state();
    state
        .hand_over_task(&task, crate::fixtures::normal("claude-2"), at(1))
        .unwrap();
    let from: agent_core::ids::AgentName = "claude-1".parse().unwrap();
    let to: agent_core::ids::AgentName = "claude-2".parse().unwrap();
    state.record_pending_team_replace(team.clone(), from.clone(), to.clone());
    std::fs::write(rig.home.state_file(), state.to_json().unwrap()).unwrap();
    let bus = AgentBus::new(
        agentbus_binary().to_str().unwrap().parse().unwrap(),
        rig.home.root().to_str().unwrap().parse().unwrap(),
    );
    bus.team_replace(&team, &from, &to).await.unwrap();
    // Retrying this completed operation now produces the real NoSuchMember response.
    rig.start().await;
    let state = rig
        .state_where("NoSuchMember permits the replacement's second run", |state| {
            state.find_task(&task).is_some_and(|found| {
                found.member.agent().name() == "claude-2"
                    && found.member.runs().len() == 2
                    && found.member.status() == TaskStatus::Running
            })
        })
        .await;
    assert!(state.pending_team_replaces().is_empty());
    assert_eq!(state.find_task(&task).unwrap().member.runs().len(), 2);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn team_replace_missing_team_stays_pending_until_the_team_exists() {
    let daemon = Daemon::new(&crate::fixtures::team_state(), None);
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.take_over(&task(2), Role::Member, Trouble::RoundOverdue).unwrap();
    block_launches(&mut owner);
    owner.start_queued().await.unwrap();
    assert_eq!(owner.parts.state.pending_team_replaces().len(), 1);
    create_bus_team(&owner).await;
    owner.start_queued().await.unwrap();
    assert!(owner.parts.state.pending_team_replaces().is_empty());
    assert!(
        owner
            .parts
            .bus
            .team_get(&team_id(0))
            .await
            .unwrap()
            .members
            .contains_key(&"claude-2".parse().unwrap())
    );
}

#[tokio::test]
async fn team_replace_closed_team_keeps_the_operation_pending() {
    let daemon = Daemon::new(&crate::fixtures::team_state(), None);
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    create_bus_team(&owner).await;
    owner.parts.bus.team_done(&team_id(0)).await.unwrap();
    owner.take_over(&task(2), Role::Member, Trouble::RoundOverdue).unwrap();
    block_launches(&mut owner);
    owner.start_queued().await.unwrap();
    assert_eq!(owner.parts.state.pending_team_replaces().len(), 1);
    owner.start_queued().await.unwrap();
    assert_eq!(owner.parts.state.pending_team_replaces().len(), 1);
}

#[tokio::test]
async fn team_replace_legacy_handover_is_migrated_after_loading_state_without_pending_field() {
    let mut state = crate::fixtures::team_state();
    state
        .hand_over_task(&task(2), crate::fixtures::normal("claude-2"), at(1))
        .unwrap();
    let mut legacy: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("pending_team_replaces");
    let daemon = Daemon::new(&state, None);
    std::fs::write(daemon.home.state_file(), serde_json::to_string(&legacy).unwrap()).unwrap();
    let loaded =
        agent_core::state::State::from_json(&std::fs::read_to_string(daemon.home.state_file()).unwrap()).unwrap();
    assert!(loaded.pending_team_replaces().is_empty());
    let (mut owner, _sinks) = daemon.bare_owner().await;
    create_bus_team(&owner).await;
    block_launches(&mut owner);
    owner.start_queued().await.unwrap();
    let bus = owner.parts.bus.team_get(&team_id(0)).await.unwrap();
    assert_eq!(
        bus.members.keys().map(ToString::to_string).collect::<Vec<_>>(),
        ["claude-2"]
    );
    assert!(owner.parts.state.pending_team_replaces().is_empty());
}
