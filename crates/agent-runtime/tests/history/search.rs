//! Literal full-text search and migration of populated version-four histories.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogRecord, LogTaskScope, LogTeamScope};
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, LogFilter, LogWindow, Reader, Store};

use super::support::{at, complete, roomy};

#[test]
fn populated_version_four_migrates_without_loss_and_indexes_old_and_new_text() {
    for seed in [8703, 8704, 8705] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let mut world = complete(seed);
        let phrase = "Zażółć gęślą jaźń \"quoted\" % * AND OR NEAR";
        world.runs[0].start.prompt = phrase.to_owned();
        let mut db = HistoryDb::open_with(&path, &MIGRATIONS[..4], roomy()).unwrap();
        db.write(&world.records()).unwrap();
        let run = &world.runs[0].start;
        db.write_logs(&[LogRecord {
            at: at(11, 0),
            module: "search-test".to_owned(),
            event: LogEvent::RunStarted,
            team: Some(LogTeamScope::new(run.team.clone(), run.rounds.clone())),
            task: Some(LogTaskScope::new(run.task.clone(), Some(run.run.clone()))),
            message: "logneedle".to_owned(),
            fields: BTreeMap::new(),
            cut: Cut::default(),
        }])
        .unwrap();
        drop(db);
        let old = Reader::open(&path).unwrap();
        let records = old.team_records(&world.team.team).unwrap();
        let logs = old.daemon_logs().unwrap();
        drop(old);
        let mut db = HistoryDb::open(&path, roomy()).unwrap();
        let reader = Reader::open(&path).unwrap();
        assert_eq!(reader.team_records(&world.team.team).unwrap(), records, "seed {seed}");
        assert_eq!(reader.daemon_logs().unwrap(), logs, "seed {seed}");
        for text in [
            phrase,
            "Zażółć",
            "gęślą",
            "step 2",
            "done",
            "logneedle",
            "AND",
            "OR",
            "NEAR",
            "\"quoted\"",
        ] {
            let filter = LogFilter {
                grep: Some(text.to_owned()),
                ..LogFilter::default()
            };
            assert_eq!(
                reader.log_rows(&filter, LogWindow::Newest(100)).unwrap(),
                logs,
                "seed {seed}: {text}"
            );
        }
        for text in ["absentneedle", "\" OR absentneedle", "", "*", "%", "\""] {
            let filter = LogFilter {
                grep: Some(text.to_owned()),
                ..LogFilter::default()
            };
            assert_eq!(
                reader.log_rows(&filter, LogWindow::Newest(100)).unwrap(),
                vec![],
                "seed {seed}: {text}"
            );
        }
        let mut next = complete(seed + 100);
        next.runs[0].start.prompt = "laterneedle".to_owned();
        db.write(&next.records()).unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        let matched: i64 = raw
            .query_row(
                "SELECT count(*) FROM history_search WHERE history_search MATCH '\"laterneedle\"'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(matched, 1, "seed {seed}");
        let indexed: i64 = raw
            .query_row(
                "SELECT count(*) FROM history_search WHERE sequence IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let total: i64 = raw
            .query_row("SELECT count(*) FROM history_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(indexed, total, "seed {seed}");
    }
}
