//! Journal transitions retain immutable workspace and snapshot identities.

use super::*;
use crate::request_workspace::{Stage, WorkspaceRecord, branch};
use agent_core::decide::worker_recovery::MAX_NO_RESULT_RETURNS;
use agent_core::decide::worker_result::WorkerEnding;

fn workspace() -> RequestWorkspace {
    let worker: AgentName = "worker-1".parse().unwrap();
    let task: TaskId = "task-20261124-120000-aaaa".parse().unwrap();
    WorkspaceRecord {
        request: RequestId::new(std::num::NonZeroU64::MIN),
        requester: "lead".parse().unwrap(),
        requester_task: "task-20261124-120000-bbbb".parse().unwrap(),
        requester_round: "round-20261124-120000-cccc".parse().unwrap(),
        requester_repo: "example".parse().unwrap(),
        worker: worker.clone(),
        worker_task: task.clone(),
        branch: branch(&worker, &task).unwrap(),
        base: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
        stage: Stage::Working,
    }
    .try_into()
    .unwrap()
}

fn run(n: u8) -> RunId {
    format!("run-20261124-12000{n}-dddd").parse().unwrap()
}
fn snapshot() -> SnapshotIdentity {
    SnapshotIdentity {
        round: workspace().record().requester_round.clone(),
        task: workspace().record().worker_task.clone(),
        number: NonZeroU32::MIN,
        commit: "fedcba9876543210fedcba9876543210fedcba98".parse().unwrap(),
        head: workspace().record().base.clone(),
    }
}
fn start(n: u8) -> Transition {
    Transition::Started {
        worker: "worker-2".parse().unwrap(),
        run: run(n),
    }
}
fn interrupted(n: u8, cause: Cause) -> Transition {
    Transition::Interrupted {
        run: run(n),
        cause,
        snapshot: snapshot(),
    }
}

#[test]
fn journal_replays_ownership_snapshots_and_independent_counters() {
    let original = workspace();
    let mut journal = Journal::new(original.clone()).unwrap();
    assert_eq!(journal.phase(), &Phase::Queued);
    for (n, cause) in [(1, Cause::Died), (2, Cause::Refused), (3, Cause::Quota)] {
        journal = journal.advance(start(n)).unwrap();
        assert_eq!(
            journal.phase(),
            &Phase::Running(Attempt {
                number: NonZeroU32::new(u32::from(n)).unwrap(),
                worker: "worker-2".parse().unwrap(),
                run: run(n)
            })
        );
        journal = journal.advance(interrupted(n, cause)).unwrap();
        assert_eq!(journal.returns(), n);
        assert_eq!(journal.wrong_try(), NonZeroU32::MIN);
        assert_eq!(journal.phase(), &Phase::Queued);
    }
    journal = journal.advance(start(4)).unwrap();
    journal = journal
        .advance(Transition::Rejected {
            run: run(4),
            wrong: vec![Wrong::EndedOtherwise(WorkerEnding::Failed)],
            snapshot: snapshot(),
        })
        .unwrap();
    assert_eq!(journal.wrong_try().get(), 2);
    assert_eq!(journal.returns(), MAX_NO_RESULT_RETURNS);
    journal = journal.advance(start(5)).unwrap();
    journal = journal.advance(interrupted(5, Cause::NoStatus)).unwrap();
    assert_eq!(journal.phase(), &Phase::Exhausted);
    assert_eq!(journal.returns(), MAX_NO_RESULT_RETURNS);
    assert_eq!(journal.workspace(), &original);
    assert_eq!(journal.events().last(), Some(&interrupted(5, Cause::NoStatus)));
    let text = serde_json::to_string(&journal).unwrap();
    assert_eq!(serde_json::from_str::<Journal>(&text).unwrap(), journal);
}

#[test]
fn invalid_transitions_never_change_the_original_journal() {
    let queued = Journal::new(workspace()).unwrap();
    let active = queued.advance(start(1)).unwrap();
    let settled = active.advance(Transition::Settled { run: run(1) }).unwrap();
    let mut foreign = snapshot();
    foreign.task = workspace().record().requester_task.clone();
    for (journal, transition) in [
        (&queued, interrupted(1, Cause::Died)),
        (&active, start(2)),
        (&active, interrupted(2, Cause::Died)),
        (&active, Transition::RestartedQueued { snapshot: snapshot() }),
        (
            &active,
            Transition::Interrupted {
                run: run(1),
                cause: Cause::RestartQueued,
                snapshot: snapshot(),
            },
        ),
        (
            &active,
            Transition::Interrupted {
                run: run(1),
                cause: Cause::Died,
                snapshot: foreign,
            },
        ),
        (
            &active,
            Transition::Rejected {
                run: run(1),
                wrong: Vec::new(),
                snapshot: snapshot(),
            },
        ),
        (&settled, start(2)),
        (&settled, Transition::Settled { run: run(1) }),
    ] {
        assert_eq!(journal.advance(transition), Err(Invalid::Transition));
    }
    assert_eq!(queued.events(), &[]);
    assert_eq!(active.events(), &[start(1)]);
}

#[test]
fn queued_restart_is_bounded_and_run_ids_cannot_be_reused() {
    let mut queued = Journal::new(workspace()).unwrap();
    for _ in 0..MAX_NO_RESULT_RETURNS {
        queued = queued
            .advance(Transition::RestartedQueued { snapshot: snapshot() })
            .unwrap();
        assert_eq!(queued.phase(), &Phase::Queued);
    }
    let exhausted = queued
        .advance(Transition::RestartedQueued { snapshot: snapshot() })
        .unwrap();
    assert_eq!(exhausted.phase(), &Phase::Exhausted);
    assert_eq!(exhausted.advance(start(1)), Err(Invalid::Transition));
    let queued = Journal::new(workspace())
        .unwrap()
        .advance(start(1))
        .unwrap()
        .advance(interrupted(1, Cause::RestartRunning))
        .unwrap();
    assert_eq!(queued.advance(start(1)), Err(Invalid::Transition));
}

#[test]
fn untrusted_json_replays_transitions_and_validates_workspace_identity() {
    let active = Journal::new(workspace()).unwrap().advance(start(1)).unwrap();
    let mut value = serde_json::to_value(&active).unwrap();
    value["events"][0]["event"] = "settled".into();
    value["events"][0].as_object_mut().unwrap().remove("worker");
    assert_eq!(
        serde_json::from_value::<Journal>(value).unwrap_err().classify(),
        serde_json::error::Category::Data
    );
    let mut value = serde_json::to_value(&active).unwrap();
    value["workspace"]["worker_task"] = workspace().record().requester_task.to_string().into();
    assert_eq!(
        serde_json::from_value::<Journal>(value).unwrap_err().classify(),
        serde_json::error::Category::Data
    );
    let ended: RequestWorkspace = WorkspaceRecord {
        stage: Stage::Ended,
        ..workspace().record().clone()
    }
    .try_into()
    .unwrap();
    assert_eq!(Journal::new(ended), Err(Invalid::Transition));
}

#[test]
fn existing_snapshot_records_supply_the_preserved_identity() {
    let record = SnapshotRecord {
        round: snapshot().round,
        task: snapshot().task,
        number: 1,
        at: "2026-11-24T12:00:00Z".parse().unwrap(),
        commit: snapshot().commit,
        head: snapshot().head,
        bytes: 123,
    };
    assert_eq!(SnapshotIdentity::try_from(&record).unwrap(), snapshot());
    assert_eq!(
        SnapshotIdentity::try_from(&SnapshotRecord { number: 0, ..record }),
        Err(Invalid::ZeroSnapshot)
    );
}
