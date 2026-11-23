//! The periodic rule through a real temporary daemon and fake agent CLI.
use super::{Daemon, at, task, team_id};
use agent_core::ids::RoundId;
use agent_core::state::{ItemPhase, State};
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;
use std::time::Duration;

/// The w6gn shape: part a merged, part b ready, no open round and no active run.
fn stalled_split() -> State {
    let mut state = crate::fixtures::solo_state();
    let id = team_id(0);
    let round = RoundId::derive(&id, 1);
    let item = "1.1".parse().unwrap();
    {
        let mut team = state.team_mut(&id).unwrap();
        team.open_round(round.clone(), Some(item), at(0)).unwrap();
        team.advance_item(
            item,
            ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task".parse().unwrap(),
            },
        )
        .unwrap();
        team.activate_split(
            agent_core::state::split::SplitPlan::new(
                item,
                ["Merged draft", "Unfinished illustration"]
                    .into_iter()
                    .map(|title| agent_core::state::split::PartDefinition {
                        title: title.into(),
                        delivers: "A useful document".into(),
                        checks: "Read the document".into(),
                        depends_on: Vec::new(),
                    })
                    .collect(),
            )
            .unwrap(),
        )
        .unwrap();
        team.deliver_round_with_pr(&round, crate::fixtures::pr(99)).unwrap();
        team.complete_delivery(&round, at(0)).unwrap();
        assert_eq!(team.open_rounds().count(), 0);
        assert_eq!(team.next_part(item).unwrap().0.to_string(), "1.1b");
    }
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member.transition(TaskStatus::Done).unwrap();
    state
}

#[tokio::test]
async fn startup_keeps_the_stalled_split_until_the_daemon_idle_window_passes() {
    let state = stalled_split();
    let expected = state.team(&team_id(0)).unwrap();
    let mut daemon = Daemon::new(&state, None);
    daemon.configure("[limits]\nidle_team_restarts = 0\n");
    let path = daemon.scenario(1, "done");
    daemon.clock.advance(time::Duration::seconds(7200));
    daemon.start().await;
    let observed = daemon
        .state_where("startup activity observation", |state| {
            state.team(&team_id(0)).is_some_and(|team| team.idle_watch().is_some())
        })
        .await;
    assert_eq!(observed.teams().len(), 1);
    assert_eq!(daemon.notifications(), Vec::<String>::new());
    daemon.clock.advance(time::Duration::seconds(7200));
    // Wait for a state change, with a bounded failure that still joins the daemon.
    let observed = tokio::time::timeout(
        Duration::from_secs(20),
        daemon.state_where("idle removal", |state| state.teams().is_empty()),
    )
    .await;
    daemon.stop().await.unwrap();
    assert_eq!(daemon.state().teams().len(), 0, "{observed:?}");
    assert!(!path.exists());
    assert!(!daemon.home.task_dir(&task(1)).exists());
    let notices = daemon.notifications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("idle"));
    let records = Reader::open(&daemon.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap();
    let decision = records
        .iter()
        .find(|r| r.fields().get("decision").is_some_and(|v| v == "idle_removal"))
        .unwrap();
    assert_eq!(decision.fields()["last_activity"], at(0).to_string());
    assert_eq!(
        decision.fields()["items"],
        serde_json::to_value(expected.items()).unwrap()
    );
    assert_eq!(
        decision.fields()["rounds"],
        serde_json::to_value(expected.rounds()).unwrap()
    );
    assert_eq!(
        decision.fields()["splits"],
        serde_json::to_value(expected.splits()).unwrap()
    );
    assert_eq!(decision.fields()["next_parts"], serde_json::json!(["1.1b"]));
    assert_eq!(
        records
            .iter()
            .filter(|r| r.event() == agent_core::log::LogEvent::TeamRemoved)
            .count(),
        1
    );
}

#[tokio::test]
async fn periodic_rule_removes_a_team_waiting_for_human_questions() {
    let mut daemon = Daemon::new(&crate::fixtures::solo_state(), None);
    daemon.configure("[limits]\nidle_team_restarts = 0\n");
    let question = daemon.questions_file(1, 1);
    let path = daemon.scenario(1, &format!("questions {}", question.display()));
    daemon.start().await;
    let waiting = daemon.status_is(1, TaskStatus::Questions).await;
    assert_eq!(waiting.find_task(&task(1)).unwrap().member.runs().len(), 1);
    daemon.clock.advance(time::Duration::seconds(7200));
    let observed = tokio::time::timeout(
        Duration::from_secs(20),
        daemon.state_where("idle removal after questions", |state| state.teams().is_empty()),
    )
    .await;
    daemon.stop().await.unwrap();
    assert_eq!(daemon.state().teams().len(), 0, "{observed:?}");
    assert!(!path.exists());
    assert_eq!(
        daemon
            .notifications()
            .iter()
            .filter(|n| n.contains("idle since"))
            .count(),
        1
    );
}

#[tokio::test]
async fn daemon_refusal_keeps_a_stalled_split_and_notifies_once_after_restart() {
    let mut daemon = Daemon::new(&stalled_split(), None);
    daemon.clock.advance(time::Duration::seconds(7200));
    daemon.start().await;
    let observed = daemon
        .state_where("startup activity observation", |state| {
            state.team(&team_id(0)).is_some_and(|team| team.idle_watch().is_some())
        })
        .await;
    assert_eq!(observed.teams().len(), 1);
    assert_eq!(daemon.notifications(), Vec::<String>::new());
    daemon.clock.advance(time::Duration::seconds(7200));
    let observed = tokio::time::timeout(
        Duration::from_secs(20),
        daemon.state_where("idle refusal", |state| {
            state
                .team(&team_id(0))
                .and_then(|t| t.idle_watch())
                .is_some_and(|w| w.refusal.is_some())
        }),
    )
    .await;
    daemon.stop().await.unwrap();
    assert!(
        daemon
            .state()
            .team(&team_id(0))
            .and_then(|t| t.idle_watch())
            .is_some_and(|w| w.refusal.is_some()),
        "no persisted idle refusal: {observed:?}"
    );
    assert_eq!(daemon.state().teams().len(), 1);
    let notices = daemon.notifications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("branches:"));
    daemon.start().await;
    daemon
        .ask(agent_core::protocol::Command::Status(
            agent_core::protocol::StatusRequest { target: None },
        ))
        .await
        .unwrap();
    daemon.stop().await.unwrap();
    assert_eq!(daemon.notifications(), notices);
}
