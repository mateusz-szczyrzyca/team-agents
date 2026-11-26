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
    let daemon = flaky_daemon();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.take_over(&task(2), Role::Member, Trouble::RoundOverdue).unwrap();
    owner.save().await.unwrap();
    block_launches(&mut owner);
    let marker = daemon.home.root().join("flaky-bus-refuse-once");
    std::fs::write(&marker, "team-replace").unwrap();
    owner.start_queued().await.unwrap();
    assert_eq!(owner.parts.state.pending_team_replaces(), &BTreeSet::new());
    assert!(!marker.exists());
}
