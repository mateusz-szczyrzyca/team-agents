//! A gate runs where no state of rounds is at hand: its event names the task, and the ledger links it to the rounds of the
//! task's latest run, the one that waits for the gate.

use agent_core::history::{EventRecord, HistoryEvent, Record};
use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
use agent_runtime::history::{Reader, Store};

use super::support::{at, complete, database};

/// The event of a gate for the task `task` of `world`.
fn gate_of(world: &super::support::World, task: &agent_core::ids::TaskId) -> Record {
    Record::Event(Box::new(EventRecord {
        at: at(2000, 0),
        team: world.team.team.clone(),
        rounds: Vec::new(),
        event: HistoryEvent::Gate {
            task: task.clone(),
            started_at: at(1990, 0),
            record: GateRecord {
                tree: GateTree::Clean,
                result: GateResult {
                    head: "1".repeat(40).parse().unwrap(),
                    finished_at: at(2000, 0),
                    outcome: GateOutcome::Passed,
                    log: "/tmp/gate-1.log".parse().unwrap(),
                },
            },
        },
    }))
}

#[test]
fn a_gate_with_no_rounds_belongs_to_the_rounds_of_the_latest_run_of_its_task() {
    for seed in 0..20 {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        let run = &world.runs[0].start;
        db.write(&[gate_of(&world, &run.task)]).unwrap();

        let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();

        let gate = read.last().unwrap();
        assert!(
            matches!(&gate.record, Record::Event(event) if event.event.kind() == agent_core::history::EventKind::Gate)
        );
        assert_eq!(gate.rounds, run.rounds, "seed={seed}");
    }
}

#[test]
fn a_gate_of_a_task_with_no_run_belongs_to_no_round() {
    let (_dir, path, mut db) = database();
    let world = complete(3);
    db.write(&world.records()).unwrap();
    let unknown = "task-20990101-000000-zzzz".parse().unwrap();
    db.write(&[gate_of(&world, &unknown)]).unwrap();

    let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();

    assert_eq!(read.last().unwrap().rounds, []);
}
