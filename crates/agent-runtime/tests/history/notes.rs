//! What the database refused is noted: when it writes again, an event says which record is missing, why, and where it
//! belonged, so a story read later can say it in the place of what is missing.

use std::sync::{Arc, Mutex};

use agent_core::history::{EventKind, HistoryEvent, Record, RoundOpening, RunStart};
use agent_runtime::history::{Noting, Reader, RecordKind, Store, WriteError};
use rusqlite::ffi;

use super::support::{at, complete, database};

/// The batches a store was given and the calls that fail.
#[derive(Debug, Default)]
struct Flaky {
    /// What was written, call by call.
    written: Arc<Mutex<Vec<Vec<Record>>>>,
    /// How many calls there were.
    calls: usize,
    /// The numbers of the calls that fail.
    failing: Vec<usize>,
}

impl Store for Flaky {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        let call = self.calls;
        self.calls += 1;
        if self.failing.contains(&call) {
            return Err(WriteError::Sqlite {
                record: RecordKind::Step,
                source: rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_FULL), None),
            });
        }
        self.written.lock().unwrap().push(records.to_vec());
        Ok(())
    }
}

/// The start of the first run of a seeded world, and a step of it.
fn run_and_step(seed: u64) -> (RunStart, Record, Record) {
    let world = complete(seed);
    let run = world.runs[0].start.clone();
    let step = |n: usize| Record::Step(Box::new(world.runs[0].steps[n].clone()));
    (run, step(0), step(1))
}

/// The `lost` events in `batches`, as (what, cause, team, rounds).
fn lost(batches: &Arc<Mutex<Vec<Vec<Record>>>>) -> Vec<(String, String, String, Vec<String>)> {
    batches
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .filter_map(|record| match record {
            Record::Event(event) => match &event.event {
                HistoryEvent::Lost { what, cause, .. } => Some((
                    what.clone(),
                    cause.clone(),
                    event.team.to_string(),
                    event.rounds.iter().map(ToString::to_string).collect(),
                )),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_record_the_database_refuses_is_noted_as_lost_with_its_team_and_rounds_when_it_writes_again() {
    for seed in 0..20 {
        let (run, first, second) = run_and_step(seed);
        let inner = Flaky {
            failing: vec![1],
            ..Flaky::default()
        };
        let written = Arc::clone(&inner.written);
        let mut store = Noting::new(inner);

        store.write(&[Record::Run(Box::new(run.clone()))]).unwrap();
        assert!(store.write(std::slice::from_ref(&first)).is_err());
        store.write(std::slice::from_ref(&second)).unwrap();

        let rounds: Vec<String> = run.rounds.iter().map(ToString::to_string).collect();
        assert_eq!(
            lost(&written),
            [(
                "step".to_owned(),
                "write_disk_full".to_owned(),
                run.team.to_string(),
                rounds
            )],
            "seed={seed}"
        );
    }
}

#[test]
fn a_record_of_a_run_nobody_saw_start_has_no_place_to_be_noted_and_is_not() {
    let (_, first, second) = run_and_step(1);
    let inner = Flaky {
        failing: vec![0],
        ..Flaky::default()
    };
    let written = Arc::clone(&inner.written);
    let mut store = Noting::new(inner);

    assert!(store.write(std::slice::from_ref(&first)).is_err());
    store.write(std::slice::from_ref(&second)).unwrap();

    assert_eq!(lost(&written), []);
}

#[test]
fn a_batch_that_fails_is_not_noted_only_the_record_tried_alone_afterwards_is() {
    let (run, first, second) = run_and_step(2);
    // The calls: the run, the batch of both steps (refused), the first step alone (refused), the second alone.
    let inner = Flaky {
        failing: vec![1, 2],
        ..Flaky::default()
    };
    let written = Arc::clone(&inner.written);
    let mut store = Noting::new(inner);
    store.write(&[Record::Run(Box::new(run))]).unwrap();

    assert!(store.write(&[first.clone(), second.clone()]).is_err());
    assert!(store.write(std::slice::from_ref(&first)).is_err());
    store.write(std::slice::from_ref(&second)).unwrap();

    assert_eq!(lost(&written).len(), 1);
}

#[test]
fn a_note_that_cannot_be_written_waits_and_goes_with_the_next_write_once() {
    let (run, first, second) = run_and_step(3);
    // The calls: the run, the refused step, the next step, its note (refused), the one after, its note.
    let inner = Flaky {
        failing: vec![1, 3],
        ..Flaky::default()
    };
    let written = Arc::clone(&inner.written);
    let mut store = Noting::new(inner);
    store.write(&[Record::Run(Box::new(run))]).unwrap();
    assert!(store.write(std::slice::from_ref(&first)).is_err());

    store.write(std::slice::from_ref(&second)).unwrap();
    assert_eq!(lost(&written), []);
    store.write(std::slice::from_ref(&second)).unwrap();
    store.write(std::slice::from_ref(&second)).unwrap();

    assert_eq!(lost(&written).len(), 1);
}

#[test]
fn a_run_the_database_refused_is_noted_in_a_real_history_and_read_back_in_the_story_of_its_round() {
    let (_dir, path, db) = database();
    let mut store = Noting::new(db);
    let world = complete(5);
    store.write(&[Record::Team(Box::new(world.team.clone()))]).unwrap();
    store
        .write(
            &world
                .rounds
                .iter()
                .map(|round: &RoundOpening| Record::Round(Box::new(round.clone())))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    // A run that names its round twice is refused whole.
    let mut bad = world.runs[0].start.clone();
    bad.rounds = vec![world.rounds[0].round.clone(), world.rounds[0].round.clone()];
    assert!(store.write(&[Record::Run(Box::new(bad.clone()))]).is_err());

    store
        .write(&[Record::Run(Box::new(world.runs[1 % world.runs.len()].start.clone()))])
        .unwrap();

    let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();
    let notes: Vec<_> = read
        .iter()
        .filter_map(|stored| match &stored.record {
            Record::Event(event) if event.event.kind() == EventKind::Lost => Some((event, &stored.rounds)),
            _ => None,
        })
        .collect();
    assert_eq!(notes.len(), 1, "{read:?}");
    let HistoryEvent::Lost { what, cause, detail } = &notes[0].0.event else {
        panic!("not a note")
    };
    assert_eq!((what.as_str(), cause.as_str()), ("run", "write_refused"));
    assert_ne!(detail, "");
    assert_eq!(notes[0].1, &[world.rounds[0].round.clone()]);
    assert_eq!(notes[0].0.at, bad.at.max(at(0, 0)));
}
