//! The schema, the pragmas and the migration mechanism: a fresh database equals the pinned file; an older database is
//! migrated forward and only forward; a newer one is not written; a file that cannot be a database is an error.

use std::path::Path;

use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, Migration, OpenError, ReadError, Reader};
use rusqlite::Connection;

use super::support::{database, roomy};

/// The pinned schema, with its comment lines left out.
fn pinned() -> String {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/history.schema.sql")).unwrap();
    let body: Vec<&str> = text.lines().filter(|line| !line.starts_with("--")).collect();
    format!("{}\n", body.join("\n").trim())
}

#[test]
fn a_fresh_database_has_the_schema_the_pinned_file_has() {
    let (_dir, path, db) = database();
    drop(db);

    let schema = Reader::open(&path).unwrap().schema().unwrap();

    assert_eq!(schema.trim(), pinned().trim());
}

#[test]
fn the_pinned_file_is_the_version_of_the_last_migration_and_every_table_of_it_is_strict() {
    let (_dir, path, db) = database();
    drop(db);
    let raw = Connection::open(&path).unwrap();

    let version: u32 = raw.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    let tables: Vec<(String, i64)> = raw
        .prepare("SELECT name, strict FROM pragma_table_list WHERE schema = 'main' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(version, MIGRATIONS.last().unwrap().version);
    assert!(pinned().starts_with(&format!("PRAGMA user_version = {version};")));
    assert_eq!(
        tables.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        [
            "changed_files",
            "commits",
            "history_record_rounds",
            "history_records",
            "launches",
            "plan",
            "rounds",
            "run_changes",
            "run_ends",
            "run_rounds",
            "run_sessions",
            "runs",
            "steps",
            "teams"
        ]
    );
    assert!(tables.iter().all(|(_, strict)| *strict == 1), "{tables:?}");
}

#[test]
fn the_database_is_in_write_ahead_log_mode() {
    let (_dir, path, db) = database();
    drop(db);
    let raw = Connection::open(&path).unwrap();

    let mode: String = raw.pragma_query_value(None, "journal_mode", |row| row.get(0)).unwrap();

    assert_eq!(mode, "wal");
}

#[test]
fn opening_a_database_again_changes_nothing() {
    let (_dir, path, db) = database();
    drop(db);
    let before = Reader::open(&path).unwrap().schema().unwrap();
    assert_eq!(before.trim(), pinned().trim());

    drop(HistoryDb::open(&path, roomy()).unwrap());

    assert_eq!(Reader::open(&path).unwrap().schema().unwrap(), before);
}

/// The first migration of the tests: a table with a row.
const FIRST: Migration = Migration {
    version: 1,
    sql: "create table kept (x TEXT NOT NULL) STRICT; INSERT INTO kept VALUES ('before');",
};

/// The second: a column that is filled.
const SECOND: Migration = Migration {
    version: 2,
    sql: "ALTER TABLE kept ADD COLUMN y TEXT; UPDATE kept SET y = 'migrated';",
};

/// The rows of `kept`.
fn kept(path: &Path) -> Vec<(String, Option<String>)> {
    let raw = Connection::open(path).unwrap();
    let has_y = raw.prepare("SELECT y FROM kept").is_ok();
    let sql = if has_y {
        "SELECT x, y FROM kept"
    } else {
        "SELECT x, NULL FROM kept"
    };
    raw.prepare(sql)
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn version(path: &Path) -> u32 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn a_database_of_an_older_version_is_migrated_forward_and_keeps_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    drop(HistoryDb::open_with(&path, &[FIRST], roomy()).unwrap());
    assert_eq!(version(&path), 1);

    drop(HistoryDb::open_with(&path, &[FIRST, SECOND], roomy()).unwrap());

    assert_eq!(version(&path), 2);
    assert_eq!(kept(&path), [("before".to_owned(), Some("migrated".to_owned()))]);
}

#[test]
fn a_fresh_database_gets_every_migration_in_order_and_each_runs_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.db");

    drop(HistoryDb::open_with(&path, &[FIRST, SECOND], roomy()).unwrap());
    drop(HistoryDb::open_with(&path, &[FIRST, SECOND], roomy()).unwrap());

    assert_eq!(version(&path), 2);
    // The first migration inserts a row: a second run of it would have made two.
    assert_eq!(kept(&path), [("before".to_owned(), Some("migrated".to_owned()))]);
}

#[test]
fn a_migration_that_fails_leaves_the_database_at_the_version_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failing.db");
    drop(HistoryDb::open_with(&path, &[FIRST], roomy()).unwrap());
    let broken = Migration {
        version: 2,
        sql: "ALTER TABLE kept ADD COLUMN z TEXT; THIS IS NOT SQL;",
    };

    let error = HistoryDb::open_with(&path, &[FIRST, broken], roomy()).unwrap_err();

    assert!(matches!(error, OpenError::Migrate { version: 2, .. }), "{error:?}");
    assert_eq!(version(&path), 1);
    let raw = Connection::open(&path).unwrap();
    assert!(
        raw.prepare("SELECT z FROM kept").is_err(),
        "the column of the failed migration is still there"
    );
}

#[test]
fn migrations_that_do_not_count_from_one_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    for (name, list, found, expected) in [
        ("from-two", vec![SECOND], 2, 1),
        (
            "gap",
            vec![
                FIRST,
                Migration {
                    version: 3,
                    sql: "SELECT 1;",
                },
            ],
            3,
            2,
        ),
        ("twice", vec![FIRST, FIRST], 1, 2),
    ] {
        let path = dir.path().join(format!("{name}.db"));

        let error = HistoryDb::open_with(&path, &list, roomy()).unwrap_err();

        assert!(
            matches!(error, OpenError::MigrationOrder { found: f, expected: e } if f == found && e == expected),
            "{name}: {error:?}"
        );
    }
}

#[test]
fn a_database_of_a_newer_version_is_refused_and_left_byte_for_byte_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("newer.db");
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("create table from_the_future (x TEXT); PRAGMA user_version = 7;")
            .unwrap();
    }
    let before = std::fs::read(&path).unwrap();

    let error = HistoryDb::open(&path, roomy()).unwrap_err();

    let latest = MIGRATIONS.last().unwrap().version;
    assert!(
        matches!(error, OpenError::Newer { found: 7, supported, .. } if supported == latest),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let siblings: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        siblings,
        ["newer.db"],
        "a journal or a log was made next to the database"
    );
}

#[test]
fn a_reader_refuses_a_newer_database_and_one_no_daemon_has_written_to() {
    let dir = tempfile::tempdir().unwrap();
    let newer = dir.path().join("newer.db");
    Connection::open(&newer)
        .unwrap()
        .execute_batch("PRAGMA user_version = 9;")
        .unwrap();
    let empty = dir.path().join("empty.db");
    Connection::open(&empty)
        .unwrap()
        .execute_batch("create table t (x TEXT);")
        .unwrap();

    let refused_newer = Reader::open(&newer).unwrap_err();
    let refused_empty = Reader::open(&empty).unwrap_err();

    assert!(
        matches!(refused_newer, ReadError::Newer { found: 9, .. }),
        "{refused_newer:?}"
    );
    assert!(matches!(refused_empty, ReadError::Empty { .. }), "{refused_empty:?}");
}

#[test]
fn a_reader_of_a_file_that_is_not_there_is_an_error_and_makes_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");

    let error = Reader::open(&path).unwrap_err();

    assert!(matches!(error, ReadError::Open { .. }), "{error:?}");
    assert!(!path.exists());
}

#[test]
fn a_file_that_is_a_directory_or_not_a_database_cannot_be_opened_for_writing() {
    let dir = tempfile::tempdir().unwrap();
    let as_directory = dir.path().join("history.db");
    std::fs::create_dir(&as_directory).unwrap();
    let as_text = dir.path().join("text.db");
    std::fs::write(
        &as_text,
        "this is not a database, it is a sentence of some length, long enough to be read as a header",
    )
    .unwrap();
    let missing_parent = dir.path().join("no-such-directory").join("history.db");

    for (name, path) in [
        ("directory", as_directory),
        ("text", as_text),
        ("missing parent", missing_parent),
    ] {
        let error = HistoryDb::open(&path, roomy()).unwrap_err();

        assert!(matches!(error, OpenError::Database { .. }), "{name}: {error:?}");
    }
}

#[test]
fn a_read_only_directory_cannot_hold_a_new_database() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let path = dir.path().join("history.db");

    let error = HistoryDb::open(&path, roomy()).unwrap_err();

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(error, OpenError::Database { .. }), "{error:?}");
}
