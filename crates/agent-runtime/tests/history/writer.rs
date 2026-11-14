//! The handle and the writer behind it: order, batches, a full queue, failures that never reach the work, the count of
//! what was lost and the report of each cause once. The stores that stall or fail stand in for the database, which is
//! the one process boundary of the writer.

use std::collections::BTreeSet;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use agent_core::cli::ActivityEvent;
use agent_core::history::{HistoryState, Record};
use agent_core::ids::RunId;
use agent_runtime::history::{
    Cause, History, HistoryDb, OpenError, Problem, RecordKind, Reporter, Setup, Store, WriteError, WriteKind,
};
use rusqlite::ffi;

use super::support::{at, complete, database, roomy, world};

/// A reporter that keeps what it is told.
#[derive(Debug, Default)]
struct Collect(Mutex<Vec<Problem>>);

impl Reporter for Collect {
    fn report(&self, problem: &Problem) {
        self.0.lock().unwrap().push(problem.clone());
    }
}

impl Collect {
    fn causes(&self) -> Vec<Cause> {
        self.0.lock().unwrap().iter().map(|problem| problem.cause).collect()
    }
}

/// The batches a store was given, in the order of its calls.
type Batches = Arc<Mutex<Vec<Vec<Record>>>>;

/// A store that keeps what it is given.
#[derive(Debug, Default)]
struct Memory(Batches);

impl Store for Memory {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        self.0.lock().unwrap().push(records.to_vec());
        Ok(())
    }
}

/// A store whose first call waits until the test lets it go, after it said it was in the call.
struct Gated {
    /// What was written.
    written: Batches,
    /// Tells the test that the first call has started.
    entered: Sender<()>,
    /// Waits for the test; `None` once the gate has been passed.
    release: Option<Receiver<()>>,
    /// Whether a batch with a record the test calls poison is refused.
    poison: Option<u32>,
}

impl Store for Gated {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        if let Some(release) = self.release.take() {
            self.entered.send(()).unwrap();
            release.recv().unwrap();
        }
        if let Some(poison) = self.poison
            && records
                .iter()
                .any(|record| matches!(record, Record::Step(step) if step.number.get() == poison))
        {
            return Err(refused());
        }
        self.written.lock().unwrap().push(records.to_vec());
        Ok(())
    }
}

/// A store that fails each call as `fail` says, by the number of the call counted from 0.
struct Failing {
    /// What was written.
    written: Batches,
    /// How many calls there were.
    calls: usize,
    /// The way call number `n` fails; `None` for a call that succeeds.
    fail: fn(usize) -> Option<i32>,
}

impl Store for Failing {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        let call = self.calls;
        self.calls += 1;
        if let Some(code) = (self.fail)(call) {
            return Err(WriteError::Sqlite {
                record: RecordKind::Step,
                source: rusqlite::Error::SqliteFailure(ffi::Error::new(code), None),
            });
        }
        self.written.lock().unwrap().push(records.to_vec());
        Ok(())
    }
}

/// An error that says the database refused a record.
fn refused() -> WriteError {
    WriteError::Sqlite {
        record: RecordKind::Step,
        source: rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_CONSTRAINT), None),
    }
}

/// A setup with this reporter and room for `capacity` records.
fn setup(reporter: &Arc<Collect>, capacity: usize) -> Setup {
    Setup {
        reporter: Arc::clone(reporter) as Arc<dyn Reporter>,
        capacity: NonZeroUsize::new(capacity).unwrap(),
    }
}

/// A step of `run`, number `n`.
fn step(run: &RunId, n: u32) -> Record {
    Record::Step(Box::new(agent_core::history::Step {
        run: run.clone(),
        number: NonZeroU32::new(n).unwrap(),
        at: at(i64::from(n), 0),
        event: ActivityEvent::Info(format!("step {n}")),
    }))
}

/// A run ID.
fn run_id(n: u32) -> RunId {
    world(u64::from(n)).runs[0].start.run.clone()
}

/// Everything a store was given, flattened.
fn written(batches: &Batches) -> Vec<Record> {
    batches.lock().unwrap().iter().flatten().cloned().collect()
}

#[test]
fn records_are_written_in_the_order_they_were_handed_in_and_flush_waits_for_them() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let started = History::start(Memory(Arc::clone(&batches)), setup(&reporter, 64)).unwrap();
    let run = run_id(1);

    for n in 1..=50 {
        started.history.record(step(&run, n));
    }
    started.history.flush();

    assert_eq!(written(&batches), (1..=50).map(|n| step(&run, n)).collect::<Vec<_>>());
    let health = started.history.health();
    assert_eq!((health.state, health.lost), (HistoryState::Writing, 0));
    started.writer.close().unwrap();
    assert_eq!(reporter.causes(), []);
}

#[test]
fn what_waits_in_the_queue_is_written_as_one_batch() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let (entered, entered_rx) = channel();
    let (release_tx, release) = channel();
    let store = Gated {
        written: Arc::clone(&batches),
        entered,
        release: Some(release),
        poison: None,
    };
    let started = History::start(store, setup(&reporter, 64)).unwrap();
    let run = run_id(2);
    started.history.record(step(&run, 1));
    entered_rx.recv().unwrap();

    for n in 2..=40 {
        started.history.record(step(&run, n));
    }
    release_tx.send(()).unwrap();
    started.history.flush();

    let sizes: Vec<usize> = batches.lock().unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, [1, 39]);
    started.writer.close().unwrap();
}

#[test]
fn a_full_queue_loses_what_does_not_fit_counts_it_and_reports_once_while_the_work_goes_on() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let (entered, entered_rx) = channel();
    let (release_tx, release) = channel();
    let store = Gated {
        written: Arc::clone(&batches),
        entered,
        release: Some(release),
        poison: None,
    };
    let started = History::start(store, setup(&reporter, 4)).unwrap();
    let run = run_id(3);
    started.history.record(step(&run, 1));
    entered_rx.recv().unwrap();

    // The writer holds record 1; four more fit in the queue; the other six do not.
    for n in 2..=11 {
        started.history.record(step(&run, n));
    }

    let health = started.history.health();
    assert_eq!(health.lost, 6);
    assert_eq!(reporter.causes(), [Cause::QueueFull]);
    release_tx.send(()).unwrap();
    started.history.flush();
    assert_eq!(written(&batches), (1..=5).map(|n| step(&run, n)).collect::<Vec<_>>());
    // The queue is empty again: records fit, and the cause is not reported twice.
    started.history.record(step(&run, 12));
    started.history.flush();
    assert_eq!(started.history.health().lost, 6);
    assert_eq!(written(&batches).last(), Some(&step(&run, 12)));
    for n in 13..=40 {
        started.history.record(step(&run, n));
    }
    assert_eq!(reporter.causes(), [Cause::QueueFull], "a cause is reported once");
    started.writer.close().unwrap();
}

#[test]
fn a_write_that_fails_loses_its_record_goes_on_and_each_cause_is_reported_once() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let store = Failing {
        written: Arc::clone(&batches),
        calls: 0,
        // Calls 0 and 1: the disk is full; 2: the database is read-only; 3 and later: fine.
        fail: |call| match call {
            0 | 1 => Some(ffi::SQLITE_FULL),
            2 => Some(ffi::SQLITE_READONLY),
            _ => None,
        },
    };
    let started = History::start(store, setup(&reporter, 16)).unwrap();
    let run = run_id(4);

    for n in 1..=5 {
        started.history.record(step(&run, n));
        started.history.flush();
    }

    assert_eq!(started.history.health().lost, 3);
    assert_eq!(written(&batches), [step(&run, 4), step(&run, 5)]);
    assert_eq!(
        reporter.causes(),
        [Cause::Write(WriteKind::DiskFull), Cause::Write(WriteKind::ReadOnly)]
    );
    assert_eq!(started.history.health().state, HistoryState::Writing);
    started.writer.close().unwrap();
}

#[test]
fn a_batch_that_fails_is_tried_again_record_by_record_so_one_bad_record_loses_only_itself() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let (entered, entered_rx) = channel();
    let (release_tx, release) = channel();
    let store = Gated {
        written: Arc::clone(&batches),
        entered,
        release: Some(release),
        poison: Some(13),
    };
    let started = History::start(store, setup(&reporter, 64)).unwrap();
    let run = run_id(5);
    started.history.record(step(&run, 1));
    entered_rx.recv().unwrap();
    for n in 2..=20 {
        started.history.record(step(&run, n));
    }

    release_tx.send(()).unwrap();
    started.history.flush();

    let kept: BTreeSet<u32> = written(&batches)
        .iter()
        .map(|record| match record {
            Record::Step(step) => step.number.get(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(kept, (1..=20).filter(|n| *n != 13).collect::<BTreeSet<u32>>());
    assert_eq!(started.history.health().lost, 1);
    assert_eq!(reporter.causes(), [Cause::Write(WriteKind::Refused)]);
    started.writer.close().unwrap();
}

#[test]
fn a_database_that_cannot_be_opened_loses_every_record_and_says_why_once() {
    let dir = tempfile::tempdir().unwrap();
    let in_the_way = dir.path().join("history.db");
    std::fs::create_dir(&in_the_way).unwrap();
    let error: OpenError = HistoryDb::open(&in_the_way, roomy()).unwrap_err();
    let reporter = Arc::new(Collect::default());

    let history = History::unavailable(&error, Arc::clone(&reporter) as Arc<dyn Reporter>);
    let run = run_id(6);
    for n in 1..=7 {
        history.record(step(&run, n));
    }
    history.step(&run, at(1, 0), ActivityEvent::Info("x".to_owned()));

    let health = history.health();
    assert_eq!(health.lost, 8);
    assert!(
        matches!(&health.state, HistoryState::Unavailable { reason } if reason.contains("history.db")),
        "{health:?}"
    );
    assert_eq!(reporter.causes(), [Cause::Unavailable]);
}

#[test]
fn a_history_that_is_off_records_nothing_and_loses_nothing() {
    let history = History::disabled();
    let run = run_id(7);

    for n in 1..=5 {
        history.record(step(&run, n));
    }
    history.step(&run, at(1, 0), ActivityEvent::Info("x".to_owned()));
    history.flush();

    let health = history.health();
    assert_eq!((health.state, health.lost), (HistoryState::Disabled, 0));
}

#[test]
fn close_writes_what_is_queued_and_afterwards_records_are_lost_and_reported_once() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let started = History::start(Memory(Arc::clone(&batches)), setup(&reporter, 64)).unwrap();
    let run = run_id(8);
    for n in 1..=10 {
        started.history.record(step(&run, n));
    }

    started.writer.close().unwrap();

    assert_eq!(written(&batches).len(), 10);
    started.history.record(step(&run, 11));
    started.history.record(step(&run, 12));
    assert_eq!(started.history.health().lost, 2);
    assert_eq!(reporter.causes(), [Cause::Unavailable]);
}

#[test]
fn steps_are_numbered_from_one_for_each_run_and_start_again_for_a_run_that_was_forgotten() {
    let reporter = Arc::new(Collect::default());
    let batches = Batches::default();
    let started = History::start(Memory(Arc::clone(&batches)), setup(&reporter, 64)).unwrap();
    let (first, second) = (run_id(9), run_id(10));

    for _ in 0..3 {
        started
            .history
            .step(&first, at(1, 0), ActivityEvent::Info("a".to_owned()));
    }
    for _ in 0..2 {
        started
            .history
            .step(&second, at(1, 0), ActivityEvent::Info("b".to_owned()));
    }
    started.history.finish_run(&first);
    started
        .history
        .step(&first, at(2, 0), ActivityEvent::Info("late".to_owned()));
    started.history.flush();

    let numbers: Vec<(RunId, u32)> = written(&batches)
        .into_iter()
        .map(|record| match record {
            Record::Step(step) => (step.run.clone(), step.number.get()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        numbers,
        [
            (first.clone(), 1),
            (first.clone(), 2),
            (first.clone(), 3),
            (second.clone(), 1),
            (second, 2),
            (first, 1)
        ]
    );
    started.writer.close().unwrap();
}

#[test]
fn the_steps_of_several_runs_recorded_at_once_keep_the_order_of_each_run() {
    const RUNS: u32 = 8;
    const STEPS: u32 = 300;
    let (_dir, path, db) = database();
    let reporter = Arc::new(Collect::default());
    let started = History::start(db, setup(&reporter, 100_000)).unwrap();
    let worlds: Vec<_> = (1..=RUNS).map(|n| complete(u64::from(n) + 100)).collect();
    for world in &worlds {
        started.history.record(Record::Team(Box::new(world.team.clone())));
        started
            .history
            .record(Record::Run(Box::new(world.runs[0].start.clone())));
    }

    std::thread::scope(|scope| {
        for world in &worlds {
            let history = started.history.clone();
            let run = world.runs[0].start.run.clone();
            scope.spawn(move || {
                for n in 1..=STEPS {
                    history.step(&run, at(i64::from(n), 0), ActivityEvent::Info(format!("{n}")));
                }
            });
        }
    });
    started.history.flush();

    let reader = agent_runtime::history::Reader::open(&path).unwrap();
    for world in &worlds {
        let steps = reader.steps(&world.runs[0].start.run).unwrap();
        let numbers: Vec<u32> = steps.iter().map(|stored| stored.value.number.get()).collect();
        let texts: Vec<String> = steps
            .iter()
            .map(|stored| match &stored.value.event {
                ActivityEvent::Info(text) => text.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(numbers, (1..=STEPS).collect::<Vec<_>>());
        assert_eq!(texts, (1..=STEPS).map(|n| n.to_string()).collect::<Vec<_>>());
    }
    assert_eq!(started.history.health().lost, 0);
    started.writer.close().unwrap();
}

#[test]
fn a_reader_that_reads_while_the_writer_writes_sees_whole_records_only() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_dir, path, db) = database();
    let reporter = Arc::new(Collect::default());
    let started = History::start(db, setup(&reporter, 100_000)).unwrap();
    let worlds: Vec<_> = (1..=80_u64).map(complete).collect();
    let done = AtomicBool::new(false);

    std::thread::scope(|scope| {
        let reading = scope.spawn(|| {
            let reader = agent_runtime::history::Reader::open(&path).unwrap();
            let mut seen = 0_usize;
            while !done.load(Ordering::Acquire) {
                for world in &worlds {
                    if let Some(team) = reader.team(&world.team.team).unwrap() {
                        assert_eq!(team.value, world.team, "a team seen half written");
                        seen += 1;
                    }
                    if let Some(changes) = reader.changes(&world.runs[0].start.run).unwrap() {
                        assert_eq!(Some(changes.value), world.runs[0].changes, "changes seen half written");
                    }
                    for row in reader.runs_of_team(&world.team.team).unwrap() {
                        let steps = reader.steps(&row.start.value.run).unwrap().len();
                        assert!(steps <= world.runs.iter().map(|run| run.steps.len()).max().unwrap_or(0));
                    }
                }
            }
            seen
        });
        for world in &worlds {
            for record in world.records() {
                started.history.record(record);
            }
        }
        started.history.flush();
        done.store(true, Ordering::Release);
        let seen = reading.join().unwrap();
        assert!(seen > 0, "the reader saw no team at all");
    });

    assert_eq!(started.history.health().lost, 0);
    started.writer.close().unwrap();
}
