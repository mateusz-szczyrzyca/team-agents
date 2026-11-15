//! Retention boundaries, atomic snapshots and durable log cursors.
use super::support::{at, complete, roomy};
use agent_core::history::{EventRecord, HistoryEvent, Record};
use agent_core::state::RoundEnd;
use agent_runtime::history::{HistoryDb, Reader, Retention, Store};

#[test]
fn retention_keeps_open_and_boundary_rounds_and_removes_all_old_owned_rows() {
    for seed in [8710, 8711, 8712] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let mut db = HistoryDb::open(&path, roomy()).unwrap();
        let mut old = complete(seed);
        old.rounds.truncate(1);
        old.runs.truncate(1);
        old.runs[0].start.rounds = vec![old.rounds[0].round.clone()];
        old.runs[0].start.prompt = "expiredretentionmarker ".repeat(30_000);
        let mut open = complete(seed + 100);
        open.rounds.truncate(1);
        open.runs.truncate(1);
        open.runs[0].start.rounds = vec![open.rounds[0].round.clone()];
        db.write(&old.records()).unwrap();
        db.write(&open.records()).unwrap();
        db.write(&[Record::Event(Box::new(EventRecord {
            at: at(2000, 0),
            team: old.team.team.clone(),
            rounds: vec![old.rounds[0].round.clone()],
            event: HistoryEvent::RoundClosed {
                round: old.rounds[0].round.clone(),
                end: RoundEnd::Merged,
            },
        }))])
        .unwrap();
        let disabled = Retention {
            closed_before: None,
            logs_before: None,
            home: dir.path().to_path_buf(),
        };
        let before = Reader::open(&path).unwrap().statistics().unwrap();
        db.retain(&disabled).unwrap();
        assert_eq!(
            Reader::open(&path).unwrap().statistics().unwrap(),
            before,
            "seed={seed}"
        );
        db.retain(&Retention {
            closed_before: Some(at(2000, 0)),
            ..disabled.clone()
        })
        .unwrap();
        assert!(
            Reader::open(&path)
                .unwrap()
                .round(&old.rounds[0].round)
                .unwrap()
                .is_some(),
            "seed={seed}"
        );
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute_batch("BEGIN").unwrap();
        let count: i64 = raw.query_row("SELECT count(*) FROM rounds", [], |r| r.get(0)).unwrap();
        db.retain(&Retention {
            closed_before: Some(at(2000, 1)),
            ..disabled
        })
        .unwrap();
        assert_eq!(
            raw.query_row("SELECT count(*) FROM rounds", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            count,
            "seed={seed}"
        );
        raw.execute_batch("COMMIT").unwrap();
        let reader = Reader::open(&path).unwrap();
        assert!(reader.round(&old.rounds[0].round).unwrap().is_none(), "seed={seed}");
        assert!(reader.team(&old.team.team).unwrap().is_none(), "seed={seed}");
        assert!(reader.run(&old.runs[0].start.run).unwrap().is_none(), "seed={seed}");
        assert!(reader.round(&open.rounds[0].round).unwrap().is_some(), "seed={seed}");
        let after = reader.statistics().unwrap();
        assert!(
            after["bytes"].as_i64().unwrap() < before["bytes"].as_i64().unwrap(),
            "seed={seed}: {before} -> {after}"
        );
        assert_eq!(
            raw.query_row(
                "SELECT count(*) FROM history_search WHERE body MATCH 'expiredretentionmarker'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "seed={seed}"
        );
        assert_owned_counts_drop(&before, &after, seed);
    }
}

/// Checks each table owning records of the deleted round.
fn assert_owned_counts_drop(before: &serde_json::Value, after: &serde_json::Value, seed: u64) {
    for table in [
        "rounds",
        "runs",
        "teams",
        "steps",
        "launches",
        "run_ends",
        "run_changes",
        "commits",
        "changed_files",
        "run_rounds",
        "plan",
        "history_records",
        "history_record_rounds",
    ] {
        let actual = after["tables"][table].as_u64().unwrap();
        assert!(
            actual < before["tables"][table].as_u64().unwrap(),
            "seed={seed} table={table}"
        );
    }
}

#[test]
fn deleting_all_log_rows_and_reopening_never_reuses_a_follow_cursor() {
    use agent_core::history::Cut;
    use agent_core::log::{LogEvent, LogRecord};
    use agent_runtime::history::{LogFilter, LogWindow};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let log = LogRecord {
        at: at(10, 0),
        module: "retention".to_owned(),
        event: LogEvent::DaemonStarted,
        team: None,
        task: None,
        message: "started".to_owned(),
        fields: std::collections::BTreeMap::new(),
        cut: Cut::default(),
    };
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(std::slice::from_ref(&log)).unwrap();
    let first = Reader::open(&path)
        .unwrap()
        .log_rows(&LogFilter::default(), LogWindow::Newest(10))
        .unwrap();
    db.retain(&Retention {
        closed_before: None,
        logs_before: Some(at(20, 0)),
        home: dir.path().to_path_buf(),
    })
    .unwrap();
    assert_eq!(
        Reader::open(&path).unwrap().statistics().unwrap()["tables"]["daemon_logs"],
        0
    );
    drop(db);
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[log]).unwrap();
    let later = Reader::open(&path)
        .unwrap()
        .log_rows(
            &LogFilter::default(),
            LogWindow::After {
                id: first[0].id(),
                limit: 10,
            },
        )
        .unwrap();
    assert_eq!(later.len(), 1);
    assert!(later[0].id() > first[0].id());
}

#[test]
fn retention_runs_through_the_single_writer_and_noting_wrapper() {
    use agent_runtime::history::{History, Noting, Setup, TracingReporter};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, roomy()).unwrap();
    let started = History::start(
        Noting::new(db),
        Setup {
            capacity: std::num::NonZeroUsize::new(1024).unwrap(),
            reporter: std::sync::Arc::new(TracingReporter),
        },
    )
    .unwrap();
    let world = complete(8720);
    for record in world.records() {
        started.history.record(record);
    }
    for round in &world.rounds {
        started.history.record(Record::Event(Box::new(EventRecord {
            at: at(2000, 0),
            team: world.team.team.clone(),
            rounds: vec![round.round.clone()],
            event: HistoryEvent::RoundClosed {
                round: round.round.clone(),
                end: RoundEnd::Merged,
            },
        })));
    }
    started.history.retain(Retention {
        closed_before: Some(at(2001, 0)),
        logs_before: None,
        home: dir.path().to_path_buf(),
    });
    started.history.flush();
    assert!(Reader::open(&path).unwrap().team(&world.team.team).unwrap().is_none());
    started.writer.close().unwrap();
}

/// A closed round with a gate path supplied by the caller.
fn gate_world(path: &std::path::Path) -> super::support::World {
    let mut world = complete(8717);
    world.rounds.truncate(1);
    world.runs.truncate(1);
    world.runs[0].start.rounds = vec![world.rounds[0].round.clone()];
    assert!(path.is_absolute());
    world
}

#[test]
fn retention_refuses_a_gate_parent_symlink_and_keeps_the_round_atomic() {
    use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("gate.log");
    std::fs::write(&victim, "must survive").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("escaped")).unwrap();
    let gate = dir.path().join("escaped/gate.log");
    let world = gate_world(&gate);
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    db.write(&[
        Record::Event(Box::new(EventRecord {
            at: at(2000, 0),
            team: world.team.team.clone(),
            rounds: vec![world.rounds[0].round.clone()],
            event: HistoryEvent::Gate {
                task: world.runs[0].start.task.clone(),
                started_at: at(1990, 0),
                record: GateRecord {
                    tree: GateTree::Clean,
                    result: GateResult {
                        head: "1".repeat(40).parse().unwrap(),
                        finished_at: at(2000, 0),
                        outcome: GateOutcome::Passed,
                        log: gate.to_str().unwrap().parse().unwrap(),
                    },
                },
            },
        })),
        Record::Event(Box::new(EventRecord {
            at: at(2001, 0),
            team: world.team.team.clone(),
            rounds: vec![world.rounds[0].round.clone()],
            event: HistoryEvent::RoundClosed {
                round: world.rounds[0].round.clone(),
                end: RoundEnd::Merged,
            },
        })),
    ])
    .unwrap();
    let before = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();
    assert!(matches!(
        db.retain(&Retention {
            closed_before: Some(at(3000, 0)),
            logs_before: None,
            home: dir.path().to_path_buf()
        }),
        Err(agent_runtime::history::RetainError::File { .. })
    ));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "must survive");
    assert_eq!(
        Reader::open(&path).unwrap().team_records(&world.team.team).unwrap(),
        before
    );
}

#[test]
fn removing_one_old_round_preserves_the_new_round_and_shared_runs_of_the_team() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut world = complete(8751);
    world.rounds.truncate(1);
    world.runs.truncate(1);
    let old = world.rounds[0].round.clone();
    let new: agent_core::ids::RoundId = "round-20261118-110640-new1".parse().unwrap();
    let mut opening = world.rounds[0].clone();
    opening.round = new.clone();
    world.rounds.push(opening);
    world.runs[0].start.rounds = vec![old.clone(), new.clone()];
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    db.write(&[Record::Event(Box::new(EventRecord {
        at: at(2000, 0),
        team: world.team.team.clone(),
        rounds: vec![old.clone()],
        event: HistoryEvent::RoundClosed {
            round: old.clone(),
            end: RoundEnd::Merged,
        },
    }))])
    .unwrap();
    let before = Reader::open(&path).unwrap().statistics().unwrap();
    db.retain(&Retention {
        closed_before: Some(at(2001, 0)),
        logs_before: None,
        home: dir.path().to_path_buf(),
    })
    .unwrap();
    let reader = Reader::open(&path).unwrap();
    assert!(reader.round(&old).unwrap().is_none());
    assert!(reader.round(&new).unwrap().is_some());
    assert_eq!(reader.team(&world.team.team).unwrap().unwrap().value, world.team);
    let mut expected = world.runs[0].start.clone();
    expected.rounds = vec![new.clone()];
    assert_eq!(reader.run(&expected.run).unwrap().unwrap().start.value, expected);
    assert_eq!(
        reader
            .steps(&expected.run)
            .unwrap()
            .iter()
            .map(|step| step.value.clone())
            .collect::<Vec<_>>(),
        world.runs[0].steps
    );
    let after = reader.statistics().unwrap();
    for (table, count) in before["tables"].as_object().unwrap() {
        if table.starts_with("history_search") {
            continue;
        }
        let removed = match table.as_str() {
            "rounds" | "run_rounds" => 1,
            "history_records" => 2,
            "history_record_rounds" => u64::try_from(world.runs[0].steps.len() + 6).unwrap(),
            _ => 0,
        };
        assert_eq!(
            after["tables"][table].as_u64().unwrap(),
            count.as_u64().unwrap() - removed,
            "{table}"
        );
    }
    assert_eq!(reader.round_records(&old).unwrap(), Vec::new());
    assert!(
        reader
            .round_records(&new)
            .unwrap()
            .iter()
            .all(|record| !record.rounds.contains(&old))
    );
}

#[test]
fn retention_shrinks_the_checkpointed_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut world = complete(8752);
    world.rounds.truncate(1);
    world.runs.truncate(1);
    world.runs[0].start.rounds = vec![world.rounds[0].round.clone()];
    world.runs[0].start.prompt = "removablelargeprompt ".repeat(30_000);
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    db.write(&[Record::Event(Box::new(EventRecord {
        at: at(2000, 0),
        team: world.team.team.clone(),
        rounds: vec![world.rounds[0].round.clone()],
        event: HistoryEvent::RoundClosed {
            round: world.rounds[0].round.clone(),
            end: RoundEnd::Merged,
        },
    }))])
    .unwrap();
    drop(db);
    let before = std::fs::metadata(&path).unwrap().len();
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.retain(&Retention {
        closed_before: Some(at(2001, 0)),
        logs_before: None,
        home: dir.path().to_path_buf(),
    })
    .unwrap();
    drop(db);
    let after = std::fs::metadata(&path).unwrap().len();
    assert!(after < before, "seed=8752: {before} -> {after}");
    assert_eq!(
        Reader::open(&path).unwrap().statistics().unwrap()["tables"]["rounds"],
        0
    );
}
