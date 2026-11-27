//! Automatic conflict logs preserve existing history while admitting the new event.

use agent_core::history::MaxTextBytes;
use agent_runtime::history::{HistoryDb, schema::MIGRATIONS};
use rusqlite::Connection;

#[test]
fn conflict_investigation_event_preserves_version_35_history_links_search_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let old: Vec<_> = MIGRATIONS.iter().copied().filter(|m| m.version <= 35).collect();
    drop(HistoryDb::open_with(&path, &old, MaxTextBytes::default()).unwrap());
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("INSERT INTO daemon_logs (id,first_at,at,level,module,event,message,fields,count,cut) VALUES (7,'2026-11-26T00:00:00Z','2026-11-26T00:00:00Z','info','agent::daemon','daemon_started','legacy searchable','{}',1,''); INSERT INTO daemon_log_rounds VALUES (7,1,'old-round');").unwrap();
    drop(raw);
    drop(HistoryDb::open(&path, MaxTextBytes::default()).unwrap());
    let raw = Connection::open(&path).unwrap();
    assert_eq!(raw.execute("INSERT INTO daemon_logs (first_at,at,level,module,event,message,fields,count,cut) VALUES ('2026-11-26T00:00:00Z','2026-11-26T00:00:00Z','info','agent::daemon','merge_conflict_investigated','answer searchable','{\"item\":\"1.1\",\"id\":1}',1,'')", []).map_err(|error| error.sqlite_error_code()), Ok(1));
    let linked: String = raw
        .query_row("SELECT round FROM daemon_log_rounds WHERE log=7", [], |r| r.get(0))
        .unwrap();
    assert_eq!(linked, "old-round");
    let hits: i64 = raw
        .query_row(
            "SELECT count(*) FROM history_search WHERE history_search MATCH 'searchable'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 2);
    let cursor: i64 = raw
        .query_row("SELECT value FROM history_cursors WHERE name='daemon_logs'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(cursor, 8);
    let broken: i64 = raw
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(broken, 0);
}

#[test]
fn a_schema_35_database_keeps_daemon_logs_links_and_search_when_migrated_to_36() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let old: Vec<_> = MIGRATIONS.iter().copied().filter(|m| m.version <= 35).collect();
    drop(HistoryDb::open_with(&path, &old, MaxTextBytes::default()).unwrap());
    let raw = Connection::open(&path).unwrap();
    let before: u32 = raw.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(before, 35);
    raw.execute_batch("INSERT INTO daemon_logs (id,first_at,at,level,module,event,message,fields,count,cut) VALUES (7,'2026-11-26T00:00:00Z','2026-11-26T00:00:00Z','info','agent::daemon','daemon_started','legacy searchable résumé','{\"kept\":true}',1,''); INSERT INTO daemon_log_rounds VALUES (7,1,'old-round');").unwrap();
    drop(raw);

    drop(HistoryDb::open(&path, MaxTextBytes::default()).unwrap());

    let raw = Connection::open(&path).unwrap();
    let after: u32 = raw.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(after, 36);
    let stored: (i64, String, String, i64, String) = raw
        .query_row("SELECT id,message,fields,count,cut FROM daemon_logs", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })
        .unwrap();
    assert_eq!(
        stored,
        (
            7,
            "legacy searchable résumé".to_owned(),
            "{\"kept\":true}".to_owned(),
            1,
            String::new()
        )
    );
    let linked: (i64, i64, String) = raw
        .query_row("SELECT log,position,round FROM daemon_log_rounds", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(linked, (7, 1, "old-round".to_owned()));
    let searchable: (i64, String) = raw
        .query_row(
            "SELECT log,body FROM history_search WHERE history_search MATCH 'searchable'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(searchable, (7, "legacy searchable résumé".to_owned()));
}
