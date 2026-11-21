//! Judgements of plan steps, written and read back through the history.

use agent_core::ids::ItemId;
use agent_core::plan::judge::JudgementState;
use agent_runtime::history::{HistoryDb, JudgementRecord, Reader, Store};

use super::support::{at, complete, roomy};

#[test]
fn judgements_are_written_and_read_back_and_latest_is_indexed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, roomy()).unwrap();

    let world = complete(1);
    db.write(&world.records()).unwrap();

    let task = world.runs[0].start.task.clone();
    let team = world.runs[0].start.team.clone();
    let run = world.runs[0].start.run.clone();

    let reader = Reader::open(&path).unwrap();
    assert_eq!(reader.step_judgements(&task).unwrap(), Vec::new());
    assert!(reader.latest_judgements(&task).unwrap().is_empty());

    let step1 = "1.1".parse::<ItemId>().unwrap();
    let step2 = "1.2".parse::<ItemId>().unwrap();

    let record1 = JudgementRecord {
        task: task.clone(),
        team: team.clone(),
        run: Some(run.clone()),
        at: at(1_000, 0),
        step: step1,
        state: JudgementState::Mismatch,
        check: Some("exists missing.txt".to_owned()),
        reason: Some("absent".to_owned()),
        duration_ms: 12,
        output_path: None,
    };

    let record2 = JudgementRecord {
        task: task.clone(),
        team: team.clone(),
        run: Some(run.clone()),
        at: at(1_500, 0),
        step: step2,
        state: JudgementState::Confirmed,
        check: Some("cargo test".to_owned()),
        reason: None,
        duration_ms: 350,
        output_path: Some("tasks/task-1/checks/1.2.log".to_owned()),
    };

    db.write_judgements(&[record1.clone(), record2.clone()]).unwrap();

    let read = reader.step_judgements(&task).unwrap();
    assert_eq!(read, vec![record1.clone(), record2.clone()]);

    let latest = reader.latest_judgements(&task).unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(latest.get(&step1), Some(&record1));
    assert_eq!(latest.get(&step2), Some(&record2));

    // Later run judges step1 again as confirmed:
    let record1_again = JudgementRecord {
        task: task.clone(),
        team: team.clone(),
        run: Some(run),
        at: at(2_000, 0),
        step: step1,
        state: JudgementState::Confirmed,
        check: Some("exists missing.txt".to_owned()),
        reason: None,
        duration_ms: 5,
        output_path: None,
    };
    db.write_judgements(std::slice::from_ref(&record1_again)).unwrap();

    let latest_updated = reader.latest_judgements(&task).unwrap();
    assert_eq!(latest_updated.get(&step1), Some(&record1_again));
}

#[test]
fn duration_overflowing_u64_max_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, roomy()).unwrap();

    let world = complete(1);
    db.write(&world.records()).unwrap();

    let task = world.runs[0].start.task.clone();
    let team = world.runs[0].start.team.clone();
    let run = world.runs[0].start.run.clone();

    let record = JudgementRecord {
        task,
        team,
        run: Some(run),
        at: at(1_000, 0),
        step: "1.1".parse::<ItemId>().unwrap(),
        state: JudgementState::Confirmed,
        check: None,
        reason: None,
        duration_ms: u64::MAX,
        output_path: None,
    };

    assert!(db.write_judgements(&[record]).is_err());
}

#[test]
fn negative_duration_is_refused_on_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, roomy()).unwrap();

    let world = complete(1);
    db.write(&world.records()).unwrap();

    let task = world.runs[0].start.task.clone();
    let team = world.runs[0].start.team.clone();
    let run = world.runs[0].start.run.clone();

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;").unwrap();
    conn.execute(
        "INSERT INTO step_judgements (task, team, run, at, step, state, check_line, reason, duration_ms, output_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            task.as_str(),
            team.as_str(),
            run.as_str(),
            "2026-11-21T01:00:00Z",
            "1.1",
            "confirmed",
            None::<String>,
            None::<String>,
            -1i64,
            None::<String>,
        ],
    )
    .unwrap();

    let reader = Reader::open(&path).unwrap();
    let err = reader.step_judgements(&task).unwrap_err();
    assert!(matches!(err, agent_runtime::history::ReadError::Value { column, .. } if column == "duration_ms"));
}

#[tokio::test]
async fn judgements_are_persisted_with_awaited_confirmation() {
    use agent_runtime::history::{DEFAULT_QUEUE, History, PersistJudgementsError, Setup, TracingReporter};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, roomy()).unwrap();

    let world = complete(1);
    db.write(&world.records()).unwrap();

    let task = world.runs[0].start.task.clone();
    let team = world.runs[0].start.team.clone();
    let run = world.runs[0].start.run.clone();

    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let history = started.history;

    let step = "1.1".parse::<ItemId>().unwrap();
    let record = JudgementRecord {
        task: task.clone(),
        team,
        run: Some(run),
        at: at(1_000, 0),
        step,
        state: JudgementState::Confirmed,
        check: Some("exists one.txt".to_owned()),
        reason: None,
        duration_ms: 15,
        output_path: None,
    };

    history.persist_judgements(vec![record.clone()]).await.unwrap();

    let reader = Reader::open(&path).unwrap();
    let read_back = reader.step_judgements(&task).unwrap();
    assert_eq!(read_back, vec![record]);

    started.writer.close().unwrap();
    let err = history
        .persist_judgements(vec![read_back[0].clone()])
        .await
        .unwrap_err();
    assert!(matches!(err, PersistJudgementsError::Unavailable));
}
