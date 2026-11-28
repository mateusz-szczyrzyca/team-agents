//! Regressions for swap accounting and runs ending during a halt.

use super::super::fixture::Daemon;
use super::super::takeover::Takeover;
use super::*;
use crate::fixtures::{solo_state, task};

#[tokio::test]
async fn harness_takeover_follows_a_human_swap() {
    let daemon = Daemon::new(&solo_state(), None);
    daemon.configure("[replacement]\nagents = [\"claude-1\"]\nreplacements = 1\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let reply = owner
        .swap_request(
            &Client::Cli,
            SwapRequest {
                task: task(1),
                agent: "claude-2".parse().unwrap(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let result = owner.take_over(&task(1), Role::Solo, Trouble::NoStatus).unwrap();
    assert_ne!(result, Takeover::Nobody);
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(
        member
            .past_holders()
            .iter()
            .map(|held| held.agent.name())
            .collect::<Vec<_>>(),
        ["claude-1", "claude-2"]
    );
    assert_eq!(member.harness_takeovers(), 1);
}
/// Leaves a naturally completed run queued for settlement while its task still appears running.
async fn swap_after_run_finished(scenario: &str) -> (super::super::Owner, Result<Reply, Failure>) {
    let daemon = Daemon::new(&solo_state(), None);
    let questions = daemon.home.task_dir(&task(1)).join("questions-1.md");
    daemon.scenario(1, &scenario.replace("QUESTIONS_PATH", &questions.to_string_lossy()));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.start_queued().await.unwrap();
    let (old_id, verdict) = tokio::time::timeout(std::time::Duration::from_secs(30), owner.runs.join_next_with_id())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let task_id = owner.running.remove(&old_id).unwrap();
    let handle = owner.runs.spawn(async move { verdict });
    owner.running.insert(handle.id(), task_id);
    let result = owner
        .swap_request(
            &Client::Cli,
            SwapRequest {
                task: task(1),
                agent: "claude-2".parse().unwrap(),
            },
        )
        .await
        .expect("a run finishing during a swap must not stop the owner");
    (owner, result)
}

#[tokio::test]
async fn swap_refuses_a_run_that_finishes_done_during_halt() {
    let (owner, reply) = swap_after_run_finished("done").await;
    assert_eq!(reply, Err(SwapRefusal::Done { task: task(1) }.into()));
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Done);
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(member.past_holders(), []);
}

#[tokio::test]
async fn swap_preserves_questions_from_a_run_finishing_during_halt() {
    let (owner, reply) = swap_after_run_finished("questions QUESTIONS_PATH").await;
    assert_eq!(reply, Ok(Reply::Applied { tasks: vec![task(1)] }));
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Questions);
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.questions().len(), 1);
    assert!(member.current_run().is_none());
}

#[tokio::test]
async fn swap_records_who_stopped_the_old_run_in_steps_and_control_history() {
    use agent_core::cli::ActivityEvent;
    use agent_core::history::{HistoryEvent, Record};
    use agent_core::protocol::{ControlAction, Target};
    use agent_runtime::history::Reader;

    for client in [
        Client::Cli,
        Client::Dashboard {
            login: "operator@example.com".parse().unwrap(),
        },
    ] {
        let daemon = Daemon::new(&solo_state(), None);
        daemon.scenario(1, "busy");
        let (owner, _sinks) = daemon.bare_owner().await;
        let settings = agent_runtime::settings::Settings::load(&daemon.home).await.unwrap();
        let opened = crate::daemon::history::open(&settings, &daemon.home).await;
        let mut owner = owner.with_history(opened.history);
        owner.start_queued().await.unwrap();
        let found = owner.parts.state.find_task(&task(1)).unwrap();
        let run = found.member.current_run().unwrap().id().clone();
        let team = found.team.id().clone();
        let reply = owner
            .swap_request(
                &client,
                SwapRequest {
                    task: task(1),
                    agent: "claude-2".parse().unwrap(),
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
        opened.writer.unwrap().close().unwrap();
        let reader = Reader::open(&daemon.home.history_file()).unwrap();
        let controls: Vec<_> = reader
            .steps(&run)
            .unwrap()
            .into_iter()
            .filter_map(|step| matches!(step.value.event, ActivityEvent::Control { .. }).then_some(step.value.event))
            .collect();
        assert_eq!(
            controls,
            [ActivityEvent::Control {
                action: ControlAction::Stop,
                by: client.clone()
            }]
        );
        let controls: Vec<_> = reader
            .team_records(&team)
            .unwrap()
            .into_iter()
            .filter_map(|row| match row.record {
                Record::Event(event) if matches!(event.event, HistoryEvent::Controlled { .. }) => Some(event.event),
                _ => None,
            })
            .collect();
        assert_eq!(
            controls,
            [HistoryEvent::Controlled {
                action: ControlAction::Stop,
                target: Target::Task(task(1)),
                by: client,
                tasks: vec![task(1)],
            }]
        );
    }
}

#[tokio::test]
async fn swap_tests_refuses_a_closed_team_with_a_queued_task() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let found = owner.parts.state.find_task(&task(1)).unwrap();
    assert_eq!(found.member.status(), TaskStatus::Queued);
    let team = found.team.id().clone();
    owner.closed.insert(team);
    let request = SwapRequest {
        task: task(1),
        agent: "claude-2".parse().unwrap(),
    };
    assert_eq!(
        owner.check_swap_refusal(&request),
        Err(SwapRefusal::Done { task: task(1) }.into())
    );
}

#[tokio::test]
async fn swap_tests_refuses_a_done_task_in_an_open_team() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let team = owner.parts.state.find_task(&task(1)).unwrap().team.id().clone();
    assert!(!owner.closed.contains(&team));
    let (_, mut member) = owner.member_mut(&task(1)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member.transition(TaskStatus::Done).unwrap();
    let request = SwapRequest {
        task: task(1),
        agent: "claude-2".parse().unwrap(),
    };
    assert_eq!(
        owner.check_swap_refusal(&request),
        Err(SwapRefusal::Done { task: task(1) }.into())
    );
}
