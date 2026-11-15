//! The remaining human controls are persisted once, and a refused control records no action.

use agent_core::history::{HistoryEvent, Record};
use agent_core::protocol::{Client, Command, ControlAction, ControlRequest, FailureKind, Reply, SayRequest, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};

#[tokio::test]
async fn pause_unpause_and_say_are_recorded_once_but_a_refused_pause_is_not() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = &tasks[0];
    rig.status_is(task, TaskStatus::Waiting).await;
    let target = Target::Task(task.clone());
    let control = ControlRequest { target: target.clone() };
    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let applied = Reply::Applied {
        tasks: vec![task.clone()],
    };
    assert_eq!(
        connection
            .request(Client::Cli, Command::Pause(control.clone()))
            .await
            .unwrap(),
        applied
    );
    rig.status_is(task, TaskStatus::Paused).await;
    let refused = connection.request(Client::Cli, Command::Pause(control.clone())).await;
    assert!(matches!(refused, Err(ClientError::Refused(failure)) if failure.kind == FailureKind::Conflict));
    assert_eq!(
        connection
            .request(Client::Cli, Command::Unpause(control))
            .await
            .unwrap(),
        applied
    );
    rig.status_is(task, TaskStatus::Waiting).await;
    assert_eq!(
        connection
            .request(
                Client::Cli,
                Command::Say(SayRequest {
                    target: target.clone(),
                    text: "Please check the result.".to_owned().try_into().unwrap(),
                })
            )
            .await
            .unwrap(),
        applied
    );
    rig.stop().await.unwrap();

    let round = rig.state().team(&team).unwrap().rounds()[0].id().clone();
    let stored = Reader::open(&rig.home.history_file())
        .unwrap()
        .round_records(&round)
        .unwrap();
    let controls: Vec<_> = stored
        .into_iter()
        .filter_map(|row| match row.record {
            Record::Event(event) if matches!(event.event, HistoryEvent::Controlled { .. }) => Some(*event),
            _ => None,
        })
        .collect();
    assert_eq!(controls.len(), 3);
    for (record, action) in controls
        .iter()
        .zip([ControlAction::Pause, ControlAction::Unpause, ControlAction::Say])
    {
        assert_eq!(record.team, team);
        assert_eq!(record.rounds, vec![round.clone()]);
        assert_eq!(
            record.event,
            HistoryEvent::Controlled {
                action,
                target: target.clone(),
                by: Client::Cli,
                tasks: vec![task.clone()],
            }
        );
    }
    assert!(controls.windows(2).all(|pair| pair[0].at <= pair[1].at));
}
