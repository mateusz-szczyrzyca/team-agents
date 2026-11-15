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
        old.runs[0].start.prompt = "x ".repeat(300_000);
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
            raw.query_row("SELECT count(*) FROM history_search WHERE body MATCH 'x'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "seed={seed}"
        );
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
}
