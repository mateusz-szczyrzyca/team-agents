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
