//! Task activity comes from validated steps in history, across every run of the task.
use super::support::{complete, database};
use agent_runtime::history::{Reader, Store};

#[test]
fn task_activity_includes_each_run_in_order_and_excludes_other_tasks() {
    for seed in [87001, 87002, 87003] {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        let reader = Reader::open(&path).unwrap();
        for run in &world.runs {
            let expected: Vec<_> = world
                .runs
                .iter()
                .filter(|other| other.start.task == run.start.task)
                .flat_map(|other| other.steps.clone())
                .collect();
            let actual: Vec<_> = reader
                .steps_of_task(&run.start.task)
                .unwrap()
                .into_iter()
                .map(|step| step.value)
                .collect();
            assert_eq!(actual, expected, "seed={seed}");
        }
        assert_eq!(
            reader
                .steps_of_task(&"task-20261115-100000-zzzz".parse().unwrap())
                .unwrap(),
            Vec::new()
        );
    }
}

use agent_core::cli::ActivityEvent;
use agent_core::history::{Record, Step};
use agent_runtime::history::{HistoryDb, OpenError};

/// Replaces the fixture's activity with the supplied events, preserving real run ownership.
fn activity_world(events: &[ActivityEvent]) -> super::support::World {
    let mut world = complete(87100);
    world.runs.truncate(1);
    let run = &mut world.runs[0];
    run.steps = events
        .iter()
        .enumerate()
        .map(|(n, event)| Step {
            run: run.start.run.clone(),
            number: u32::try_from(n + 1).unwrap().try_into().unwrap(),
            at: super::support::at(1000, 0),
            event: event.clone(),
        })
        .collect();
    world
}

#[test]
fn append_activity_writes_one_record_per_event_in_order() {
    let (_dir, path, mut db) = database();
    let events = [
        ActivityEvent::Text("hello żółć".to_owned()),
        ActivityEvent::Command("cargo test".to_owned()),
        ActivityEvent::FileEdit {
            path: "src/lib.rs".to_owned(),
        },
        ActivityEvent::BusCall {
            tool: "notebook_append".to_owned(),
            arguments: "{}".to_owned(),
        },
        ActivityEvent::ToolCall {
            name: "Read".to_owned(),
            summary: "a file".to_owned(),
        },
        ActivityEvent::Error("boom".to_owned()),
        ActivityEvent::Info("model x".to_owned()),
    ];
    let world = activity_world(&events);
    db.write(&world.records()).unwrap();
    let read = Reader::open(&path)
        .unwrap()
        .steps_of_task(&world.runs[0].start.task)
        .unwrap();
    assert_eq!(
        read.into_iter().map(|row| row.value).collect::<Vec<_>>(),
        world.runs[0].steps
    );
}

#[test]
fn the_activity_log_grows_across_stores_and_is_kept_per_task() {
    let (_dir, path, mut db) = database();
    let world = activity_world(&[ActivityEvent::Text("one".to_owned())]);
    let other = complete(87101);
    db.write(&world.records()).unwrap();
    db.write(&other.records()).unwrap();
    drop(db);
    let mut second = HistoryDb::open(&path, super::support::roomy()).unwrap();
    let step = Step {
        number: 2.try_into().unwrap(),
        event: ActivityEvent::Text("two".to_owned()),
        ..world.runs[0].steps[0].clone()
    };
    second.write(&[Record::Step(Box::new(step.clone()))]).unwrap();
    let reader = Reader::open(&path).unwrap();
    let read = reader.steps_of_task(&world.runs[0].start.task).unwrap();
    assert_eq!(
        read.into_iter().map(|r| r.value).collect::<Vec<_>>(),
        [world.runs[0].steps[0].clone(), step]
    );
    assert_eq!(
        reader
            .steps_of_task(&other.runs[0].start.task)
            .unwrap()
            .into_iter()
            .map(|r| r.value)
            .collect::<Vec<_>>(),
        other.runs[0].steps
    );
}

#[test]
fn append_activity_cuts_a_long_text_on_a_character_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, super::support::limit(4096)).unwrap();
    let world = activity_world(&[ActivityEvent::Text(format!("a{}", "ż".repeat(4096)))]);
    db.write(&world.records()).unwrap();
    let read = Reader::open(&path)
        .unwrap()
        .steps_of_task(&world.runs[0].start.task)
        .unwrap();
    assert_eq!(
        read[0].value.event,
        ActivityEvent::Text(format!("a{}", "ż".repeat(2047)))
    );
    assert!(read[0].cut.has("text"));
}

#[test]
fn append_activity_keeps_a_text_at_the_limit_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, super::support::limit(4096)).unwrap();
    let world = activity_world(&[ActivityEvent::Text("a".repeat(4096))]);
    db.write(&world.records()).unwrap();
    let read = Reader::open(&path)
        .unwrap()
        .steps_of_task(&world.runs[0].start.task)
        .unwrap();
    assert_eq!(read[0].value, world.runs[0].steps[0]);
    assert!(!read[0].cut.has("text"));
}

#[test]
fn append_activity_reports_a_log_that_cannot_be_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    std::fs::create_dir(&path).unwrap();
    assert!(
        matches!(HistoryDb::open(&path, super::support::roomy()), Err(OpenError::Database { path: failed, .. }) if failed == path)
    );
}
