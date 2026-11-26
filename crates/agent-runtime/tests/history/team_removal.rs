//! A history predating removal keeps its log and admits the new removal event.

use agent_runtime::history::{HistoryDb, Reader};

use super::support::roomy;

#[test]
fn a_version_19_history_keeps_its_log_and_accepts_team_removal_after_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-team-removal.sql"))
        .unwrap();

    drop(HistoryDb::open(&path, roomy()).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute(
        "INSERT INTO daemon_logs (id, first_at, at, level, module, event, message, fields, count, cut) VALUES (8, '2026-11-23T01:00:00Z', '2026-11-23T01:00:00Z', 'info', 'agent::daemon', 'team_removed', 'removed team', '{}', 1, '')",
        [],
    ).expect("the upgraded history accepts team_removed");
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 33);
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter().map(|entry| entry.event().as_str()).collect::<Vec<_>>(),
        vec!["team_changed", "team_removed"]
    );
    let cursor: i64 = connection
        .query_row(
            "SELECT value FROM history_cursors WHERE name = 'daemon_logs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cursor, 8);
    let search: Vec<String> = connection
        .prepare("SELECT body FROM history_search WHERE log IS NOT NULL ORDER BY log")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(search, vec!["existing team", "removed team"]);
}
