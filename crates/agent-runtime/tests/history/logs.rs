//! The daemon's own log in `history.db` (`daemon_logs` and `daemon_log_rounds`): schema migration 4, writing and
//! reading across global, team, round, task and run scopes, coalescing repeated warnings, enforcing SQLite and reader
//! invariants, upgrading older databases, seeded sequence properties, secret redaction, and non-counting of log drops
//! in `HistoryHealth.lost`.

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use agent_core::config::Secrets;
use agent_core::history::{Cut, Record};
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEntry, LogEvent, LogLevel, LogRecord, LogTaskScope, LogTeamScope};
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{
    DEFAULT_QUEUE, History, HistoryDb, Noting, Problem, ReadError, Reader, RecordKind, Reporter, Setup, Store,
    WriteError,
};
use rand::rngs::Xoshiro256PlusPlus;
use rand::{RngExt, SeedableRng};
use rusqlite::{Connection, params};
use serde_json::json;

use super::support::{at, complete, database, limit, roomy, some_text};

fn team_a() -> TeamId {
    "team-20260413-090000-aaa1".parse().unwrap()
}

fn team_b() -> TeamId {
    "team-20260413-090500-bbb2".parse().unwrap()
}

fn round_a(pos: u32) -> RoundId {
    RoundId::derive(&team_a(), pos)
}

fn task_a() -> TaskId {
    "task-20260413-090100-ccc3".parse().unwrap()
}

fn task_b() -> TaskId {
    "task-20260413-090200-ddd4".parse().unwrap()
}

fn run_a(n: u32) -> RunId {
    format!("run-20260413-09010{n}-eee{n}").parse().unwrap()
}

#[derive(Debug, Default)]
struct Reports(Mutex<Vec<Problem>>);

impl Reporter for Reports {
    fn report(&self, problem: &Problem) {
        self.0.lock().unwrap().push(problem.clone());
    }
}

#[derive(Debug)]
struct FailingLogStore;

impl Store for FailingLogStore {
    fn write(&mut self, _records: &[Record]) -> Result<(), WriteError> {
        Ok(())
    }

    fn write_logs(&mut self, _records: &[LogRecord]) -> Result<(), WriteError> {
        Err(WriteError::Rule {
            record: RecordKind::Event,
            rule: "simulated log write failure",
        })
    }
}

#[test]
fn daemon_logs_round_trip_for_global_team_round_task_and_run_scopes() {
    let (_dir, path, mut db) = database();

    let global = LogRecord {
        at: at(10, 0),
        module: "agent::daemon::server".to_owned(),
        event: LogEvent::RequestRefused,
        team: None,
        task: None,
        message: "unknown team in request".to_owned(),
        fields: BTreeMap::from([("reason".to_owned(), json!("no such team"))]),
        cut: Cut::default(),
    };
    let team_only = LogRecord {
        at: at(20, 0),
        module: "agent::daemon::owner".to_owned(),
        event: LogEvent::TeamChanged,
        team: Some(LogTeamScope::new(team_a(), [])),
        task: None,
        message: "team started".to_owned(),
        fields: BTreeMap::from([("status".to_owned(), json!("running"))]),
        cut: Cut::default(),
    };
    let team_and_round = LogRecord {
        at: at(30, 0),
        module: "agent::daemon::team::assign".to_owned(),
        event: LogEvent::DecisionTaken,
        team: Some(LogTeamScope::new(team_a(), [round_a(1)])),
        task: None,
        message: "assigned item to agent".to_owned(),
        fields: BTreeMap::from([
            ("decision".to_owned(), json!("assign_item")),
            ("input".to_owned(), json!({"item": "1.1"})),
            ("result".to_owned(), json!({"agent": "claude-1"})),
        ]),
        cut: Cut::default(),
    };
    let solo_task = LogRecord {
        at: at(40, 0),
        module: "agent::daemon::gate".to_owned(),
        event: LogEvent::GateTimedOut,
        team: None,
        task: Some(LogTaskScope::new(task_b(), None)),
        message: "gate exceeded timeout".to_owned(),
        fields: BTreeMap::from([("timeout_minutes".to_owned(), json!(30))]),
        cut: Cut::default(),
    };
    let run_multi_round = LogRecord {
        at: at(50, 0),
        module: "agent_runtime::run".to_owned(),
        event: LogEvent::CliOutputUnreadable,
        team: Some(LogTeamScope::new(team_a(), [round_a(2), round_a(1)])),
        task: Some(LogTaskScope::new(task_a(), Some(run_a(1)))),
        message: "model CLI line is not valid JSON".to_owned(),
        fields: BTreeMap::from([("line".to_owned(), json!("bad { json"))]),
        cut: Cut::default(),
    };

    db.write_logs(&[
        global.clone(),
        team_only.clone(),
        team_and_round.clone(),
        solo_task.clone(),
        run_multi_round.clone(),
    ])
    .unwrap();

    let reader = Reader::open(&path).unwrap();
    let all = reader.daemon_logs().unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(
        all.iter().map(agent_core::log::LogEntry::id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert!(all.iter().all(|entry| entry.count() == NonZeroU64::new(1).unwrap()));
    assert!(all.iter().all(|entry| entry.first_at() == entry.at()));

    assert_eq!(all[0].event(), LogEvent::RequestRefused);
    assert_eq!(all[0].level(), LogLevel::Warn);
    assert_eq!(all[0].team(), None);
    assert_eq!(all[0].rounds(), &[]);
    assert_eq!(all[0].task(), None);
    assert_eq!(all[0].run(), None);

    assert_eq!(all[4].event(), LogEvent::CliOutputUnreadable);
    assert_eq!(all[4].team(), Some(&team_a()));
    assert_eq!(all[4].rounds(), &[round_a(1), round_a(2)]);
    assert_eq!(all[4].task(), Some(&task_a()));
    assert_eq!(all[4].run(), Some(&run_a(1)));
    assert_eq!(all[4].fields().get("line"), Some(&json!("bad { json")));

    let for_team_a = reader.daemon_logs_for_team(&team_a()).unwrap();
    assert_eq!(for_team_a.iter().map(LogEntry::id).collect::<Vec<_>>(), vec![2, 3, 5]);
    assert_eq!(reader.daemon_logs_for_team(&team_b()).unwrap(), Vec::<LogEntry>::new());

    let for_round_1 = reader.daemon_logs_for_round(&round_a(1)).unwrap();
    assert_eq!(for_round_1.iter().map(LogEntry::id).collect::<Vec<_>>(), vec![3, 5]);
    let for_round_2 = reader.daemon_logs_for_round(&round_a(2)).unwrap();
    assert_eq!(for_round_2.iter().map(LogEntry::id).collect::<Vec<_>>(), vec![5]);

    let for_task_a = reader.daemon_logs_for_task(&task_a()).unwrap();
    assert_eq!(for_task_a.iter().map(LogEntry::id).collect::<Vec<_>>(), vec![5]);
    let for_task_b = reader.daemon_logs_for_task(&task_b()).unwrap();
    assert_eq!(for_task_b.iter().map(LogEntry::id).collect::<Vec<_>>(), vec![4]);

    let for_run_1 = reader.daemon_logs_for_run(&run_a(1)).unwrap();
    assert_eq!(for_run_1.iter().map(LogEntry::id).collect::<Vec<_>>(), vec![5]);
}

#[test]
fn the_same_warning_five_times_is_one_row_with_count_five_and_near_duplicates_stay_separate() {
    let (_dir, path, mut db) = database();

    let base_warn = LogRecord {
        at: at(10, 0),
        module: "agent::daemon::owner::teams".to_owned(),
        event: LogEvent::BusDown,
        team: Some(LogTeamScope::new(team_a(), [round_a(1)])),
        task: Some(LogTaskScope::new(task_a(), Some(run_a(1)))),
        message: "agentbus inbox call failed".to_owned(),
        fields: BTreeMap::from([("operation".to_owned(), json!("inbox"))]),
        cut: Cut::default(),
    };

    for step in 0..5 {
        let mut repeated = base_warn.clone();
        repeated.at = at(10 + step * 5, 0);
        db.write_logs(&[repeated]).unwrap();
    }

    let reader = Reader::open(&path).unwrap();
    let entries = reader.daemon_logs().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].count(), NonZeroU64::new(5).unwrap());
    assert_eq!(entries[0].first_at(), at(10, 0));
    assert_eq!(entries[0].at(), at(30, 0));

    // Nearest non-duplicates: each differs in exactly one dimension and must NOT coalesce.
    let mut diff_event = base_warn.clone();
    diff_event.at = at(40, 0);
    diff_event.event = LogEvent::GitCallFailed;

    let mut diff_team = base_warn.clone();
    diff_team.at = at(41, 0);
    diff_team.team = Some(LogTeamScope::new(team_b(), [round_a(1)]));

    let mut diff_rounds = base_warn.clone();
    diff_rounds.at = at(42, 0);
    diff_rounds.team = Some(LogTeamScope::new(team_a(), [round_a(1), round_a(2)]));

    let mut diff_task = base_warn.clone();
    diff_task.at = at(43, 0);
    diff_task.task = Some(LogTaskScope::new(task_b(), Some(run_a(1))));

    let mut diff_run = base_warn.clone();
    diff_run.at = at(44, 0);
    diff_run.task = Some(LogTaskScope::new(task_a(), Some(run_a(2))));

    let mut diff_message = base_warn.clone();
    diff_message.at = at(45, 0);
    diff_message.message = "agentbus send call failed".to_owned();

    let mut diff_fields = base_warn.clone();
    diff_fields.at = at(46, 0);
    diff_fields.fields.insert("operation".to_owned(), json!("send"));

    db.write_logs(&[
        diff_event,
        diff_team,
        diff_rounds,
        diff_task,
        diff_run,
        diff_message,
        diff_fields,
    ])
    .unwrap();

    // Non-warnings (error, info, debug) repeated twice never coalesce.
    for (idx, event) in [
        LogEvent::StateSaveFailed,
        LogEvent::RunStarted,
        LogEvent::OutsideCallCompleted,
    ]
    .into_iter()
    .enumerate()
    {
        for rep in 0..2_i64 {
            let mut non_warn = base_warn.clone();
            non_warn.at = at(60 + i64::try_from(idx).unwrap() * 10 + rep, 0);
            non_warn.event = event;
            db.write_logs(&[non_warn]).unwrap();
        }
    }

    let all = reader.daemon_logs().unwrap();
    // 1 coalesced warning + 7 near-duplicates + 6 non-warnings = 14 rows.
    assert_eq!(all.len(), 14);
    assert_eq!(all[0].count(), NonZeroU64::new(5).unwrap());
    for entry in &all[1..] {
        assert_eq!(entry.count(), NonZeroU64::new(1).unwrap(), "{entry:?}");
    }
}

#[test]
fn every_rule_of_daemon_logs_is_checked_by_sqlite() {
    let (_dir, path, mut db) = database();
    db.write_logs(&[LogRecord {
        at: at(10, 0),
        module: "agent::daemon".to_owned(),
        event: LogEvent::RunStarted,
        team: Some(LogTeamScope::new(team_a(), [round_a(1)])),
        task: Some(LogTaskScope::new(task_a(), Some(run_a(1)))),
        message: "run started".to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    }])
    .unwrap();
    drop(db);

    assert_eq!(Reader::open(&path).unwrap().daemon_logs().unwrap().len(), 1);

    let raw = Connection::open(&path).unwrap();
    raw.pragma_update(None, "foreign_keys", true).unwrap();

    // SQLite CHECK constraints reject invalid rows at write time.
    let bad_sql_statements = [
        // Unknown level
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'trace', 'm', 'run_started', 'msg', '{}', 1, '')",
        // Unknown event
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'info', 'm', 'made_up', 'msg', '{}', 1, '')",
        // Mismatched level and event
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'info', 'm', 'cli_output_unreadable', 'msg', '{}', 1, '')",
        // Zero count
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'm', 'bus_down', 'msg', '{}', 0, '')",
        // Count > 1 on non-warn
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'info', 'm', 'run_started', 'msg', '{}', 2, '')",
        // first_at > at
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:01:00Z', '2026-04-13T09:00:00Z', 'warn', 'm', 'bus_down', 'msg', '{}', 1, '')",
        // run without task
        "INSERT INTO daemon_logs (first_at, at, level, module, event, run, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'm', 'bus_down', 'run-20260413-090101-eee1', 'msg', '{}', 1, '')",
        // fields not a JSON object
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'm', 'bus_down', 'msg', '[]', 1, '')",
        // position 0 in daemon_log_rounds
        "INSERT INTO daemon_log_rounds (log, position, round) VALUES (1, 0, 'round-20260413-090000-aaa2')",
        // duplicate round for same log
        "INSERT INTO daemon_log_rounds (log, position, round) VALUES (1, 2, (SELECT round FROM daemon_log_rounds WHERE log = 1 AND position = 1))",
    ];
    for sql in bad_sql_statements {
        assert!(raw.execute(sql, []).is_err(), "SQLite should refuse: {sql}");
    }
}

#[test]
fn every_rule_of_daemon_logs_is_checked_again_when_read() {
    let (_dir, path, db) = database();
    drop(db);

    let raw = Connection::open(&path).unwrap();
    raw.pragma_update(None, "foreign_keys", false).unwrap();
    raw.execute_batch(
        "DROP TABLE IF EXISTS daemon_log_rounds;
         DROP TABLE IF EXISTS daemon_logs;
         CREATE TABLE daemon_logs (
             id INTEGER PRIMARY KEY,
             first_at TEXT NOT NULL,
             at TEXT NOT NULL,
             level TEXT NOT NULL,
             module TEXT NOT NULL,
             event TEXT NOT NULL,
             team TEXT,
             task TEXT,
             run TEXT,
             message TEXT NOT NULL,
             fields TEXT NOT NULL,
             count INTEGER NOT NULL,
             cut TEXT NOT NULL
         );
         CREATE TABLE daemon_log_rounds (
             log INTEGER NOT NULL,
             position INTEGER NOT NULL,
             round TEXT NOT NULL
         );",
    )
    .unwrap();

    let tampered_cases: &[(&str, &str, &str)] = &[
        // Empty module
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', '', 'bus_down', NULL, NULL, NULL, 'm', '{}', 1, '')",
            "daemon_logs",
            "module",
        ),
        // Level mismatch
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'info', 'mod', 'bus_down', NULL, NULL, NULL, 'm', '{}', 1, '')",
            "daemon_logs",
            "level",
        ),
        // Non-warn repeated
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'info', 'mod', 'run_started', NULL, NULL, NULL, 'm', '{}', 2, '')",
            "daemon_logs",
            "count",
        ),
        // first_at > at
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:01:00Z', '2026-04-13T09:00:00Z', 'warn', 'mod', 'bus_down', NULL, NULL, NULL, 'm', '{}', 1, '')",
            "daemon_logs",
            "first_at",
        ),
        // run without task
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'mod', 'bus_down', NULL, NULL, 'run-20260413-090101-eee1', 'm', '{}', 1, '')",
            "daemon_logs",
            "run",
        ),
        // unknown cut field
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'mod', 'bus_down', NULL, NULL, NULL, 'm', '{}', 1, 'prompt')",
            "daemon_logs",
            "cut",
        ),
        // rounds without team
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'mod', 'bus_down', NULL, NULL, NULL, 'm', '{}', 1, '');
             INSERT INTO daemon_log_rounds VALUES (1, 1, 'round-20260413-090000-aaa2');",
            "daemon_logs",
            "team",
        ),
        // round position gap
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'mod', 'bus_down', 'team-20260413-090000-aaa1', NULL, NULL, 'm', '{}', 1, '');
             INSERT INTO daemon_log_rounds VALUES (1, 2, 'round-20260413-090000-aaa2');",
            "daemon_log_rounds",
            "position",
        ),
        // unsorted rounds
        (
            "INSERT INTO daemon_logs VALUES (1, '2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', 'mod', 'bus_down', 'team-20260413-090000-aaa1', NULL, NULL, 'm', '{}', 1, '');
             INSERT INTO daemon_log_rounds VALUES (1, 1, 'round-20260413-090000-aaa3');
             INSERT INTO daemon_log_rounds VALUES (1, 2, 'round-20260413-090000-aaa2');",
            "daemon_log_rounds",
            "round",
        ),
    ];

    let reader = Reader::open(&path).unwrap();
    for &(setup_sql, expected_table, expected_column) in tampered_cases {
        raw.execute_batch("DELETE FROM daemon_log_rounds; DELETE FROM daemon_logs;")
            .unwrap();
        raw.execute_batch(setup_sql).unwrap();
        let err = reader.daemon_logs().unwrap_err();
        assert!(
            matches!(
                &err,
                ReadError::Value { table, column, .. }
                if *table == expected_table && *column == expected_column
            ),
            "expected Value error on {expected_table}.{expected_column} for {setup_sql}, got {err:?}"
        );
    }
}

#[test]
fn older_databases_read_empty_daemon_logs_and_upgrade_to_version_four_without_loss() {
    for old_version in 1..=3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let world = complete(u64::try_from(old_version).unwrap());

        let mut old_db = HistoryDb::open_with(&path, &MIGRATIONS[..old_version], roomy()).unwrap();
        for record in world.records() {
            if old_version < 2 && matches!(record, Record::Session(_)) {
                continue;
            }
            if old_version < 3 && matches!(record, Record::Event(_)) {
                continue;
            }
            old_db.write(&[record]).unwrap();
        }
        old_db.write_logs(&[]).unwrap();
        let refused = old_db.write_logs(&[LogRecord {
            at: at(10, 0),
            module: "agent::daemon".to_owned(),
            event: LogEvent::BusDown,
            team: None,
            task: None,
            message: "bus down".to_owned(),
            fields: BTreeMap::new(),
            cut: Cut::default(),
        }]);
        assert!(matches!(refused, Err(WriteError::Rule { .. })));
        drop(old_db);

        let old_reader = Reader::open(&path).unwrap();
        assert_eq!(old_reader.daemon_logs().unwrap(), Vec::<LogEntry>::new());
        assert_eq!(
            old_reader.daemon_logs_for_team(&world.team.team).unwrap(),
            Vec::<LogEntry>::new()
        );
        assert_eq!(
            old_reader.daemon_logs_for_round(&world.rounds[0].round).unwrap(),
            Vec::<LogEntry>::new()
        );
        drop(old_reader);

        // Opening with HistoryDb::open migrates to the pinned schema without losing existing data.
        let mut upgraded = HistoryDb::open(&path, roomy()).unwrap();
        upgraded
            .write_logs(&[LogRecord {
                at: at(99, 0),
                module: "agent::daemon".to_owned(),
                event: LogEvent::TeamChanged,
                team: Some(LogTeamScope::new(
                    world.team.team.clone(),
                    [world.rounds[0].round.clone()],
                )),
                task: None,
                message: "upgraded log entry".to_owned(),
                fields: BTreeMap::new(),
                cut: Cut::default(),
            }])
            .unwrap();
        drop(upgraded);

        let reader = Reader::open(&path).unwrap();
        assert!(reader.team(&world.team.team).unwrap().is_some());
        let logs = reader.daemon_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message(), "upgraded log entry");

        let raw = Connection::open(&path).unwrap();
        let version: u32 = raw.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(
            format!("PRAGMA user_version = {version};"),
            include_str!("../../../../docs/history.schema.sql")
                .lines()
                .find(|line| line.starts_with("PRAGMA user_version = "))
                .unwrap()
        );
    }
}

#[test]
fn seeded_sequences_of_daemon_logs_preserve_order_and_coalesced_counts() {
    for seed in 0..20_u64 {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let (_dir, path, mut db) = database();
        let mut generated = Vec::new();

        for step in 0..25_i64 {
            let event = LogEvent::ALL[rng.random_range(0..LogEvent::ALL.len())];
            let (team, message, fields) = if event.level() == LogLevel::Warn && rng.random_bool(0.5) {
                (
                    Some(LogTeamScope::new(team_a(), [round_a(1)])),
                    "repeated warning zażółć".to_owned(),
                    BTreeMap::from([("kind".to_owned(), json!("steady"))]),
                )
            } else {
                let team = rng
                    .random_bool(0.6)
                    .then(|| LogTeamScope::new(team_a(), [round_a(1), round_a(2)]));
                (
                    team,
                    some_text(&mut rng, 40),
                    BTreeMap::from([
                        ("n".to_owned(), json!(step)),
                        ("text".to_owned(), json!(some_text(&mut rng, 20))),
                    ]),
                )
            };
            let task = rng
                .random_bool(0.5)
                .then(|| LogTaskScope::new(task_a(), rng.random_bool(0.5).then(|| run_a(1))));
            generated.push(LogRecord {
                at: at(step * 2, 0),
                module: "agent::daemon".to_owned(),
                event,
                team,
                task,
                message,
                fields,
                cut: Cut::default(),
            });
        }

        let mut cursor = 0;
        while cursor < generated.len() {
            let take = rng.random_range(1..=4).min(generated.len() - cursor);
            db.write_logs(&generated[cursor..cursor + take]).unwrap();
            cursor += take;
        }
        drop(db);

        let entries = Reader::open(&path).unwrap().daemon_logs().unwrap();
        let total_count: u64 = entries.iter().map(|entry| entry.count().get()).sum();
        assert_eq!(total_count, u64::try_from(generated.len()).unwrap(), "seed={seed}");
        let ids: Vec<i64> = entries.iter().map(agent_core::log::LogEntry::id).collect();
        assert_eq!(
            ids,
            (1..=i64::try_from(entries.len()).unwrap()).collect::<Vec<_>>(),
            "seed={seed}"
        );
        for entry in &entries {
            assert!(entry.first_at() <= entry.at(), "seed={seed}: {entry:?}");
            if entry.level() != LogLevel::Warn {
                assert_eq!(entry.count(), NonZeroU64::new(1).unwrap(), "seed={seed}: {entry:?}");
            }
        }
    }
}

#[test]
fn history_record_log_redacts_secrets_and_does_not_count_log_drops_as_lost_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, limit(64)).unwrap();
    let secrets = Secrets::parse(
        "github_token = 'ghp_secret_token_123'\n\
         [agent_env]\n\
         API_KEY = 'sk-agent-secret-456'\n\
         [notification_env]\n\
         NTFY_KEY = 'ntfy-secret-789'\n",
        0o600,
    )
    .unwrap();
    let reports = Arc::new(Reports::default());
    let started = History::start_with_secrets(
        Noting::with_secrets(db, secrets.clone()),
        Setup {
            capacity: DEFAULT_QUEUE,
            reporter: Arc::clone(&reports) as Arc<dyn Reporter>,
        },
        secrets,
    )
    .unwrap();

    started.history.record_log(LogRecord {
        at: at(10, 0),
        module: "agent_runtime::github".to_owned(),
        event: LogEvent::GithubCallFailed,
        team: Some(LogTeamScope::new(team_a(), [round_a(1)])),
        task: None,
        message: "auth failed with ghp_secret_token_123 and sk-agent-secret-456".to_owned(),
        fields: BTreeMap::from([(
            "headers".to_owned(),
            json!({"notify": "ntfy-secret-789", "nested": ["ghp_secret_token_123"]}),
        )]),
        cut: Cut::default(),
    });
    started.writer.close().unwrap();
    assert_eq!(started.history.health().lost, 0);

    let entries = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].message(), "auth failed with [REDACTED] and [REDACTED]");
    let headers = entries[0].fields().get("headers").unwrap();
    assert_eq!(headers, &json!({"notify": "[REDACTED]", "nested": ["[REDACTED]"]}));

    // Raw bytes of history.db must not contain any of the configured secrets.
    let raw = Connection::open(&path).unwrap();
    let _: () = raw
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", params![], |_| Ok(()))
        .unwrap();
    drop(raw);
    let db_bytes = std::fs::read(&path).unwrap();
    let db_text = String::from_utf8_lossy(&db_bytes);
    for secret in ["ghp_secret_token_123", "sk-agent-secret-456", "ntfy-secret-789"] {
        assert!(!db_text.contains(secret), "secret {secret} leaked into history.db");
    }

    // Failed or unavailable daemon-log writes do NOT increment HistoryHealth.lost (Question 2 Verdict: a).
    let failing = History::start(
        FailingLogStore,
        Setup {
            capacity: DEFAULT_QUEUE,
            reporter: Arc::new(Reports::default()),
        },
    )
    .unwrap();
    failing.history.record_log(LogRecord {
        at: at(20, 0),
        module: "agent::daemon".to_owned(),
        event: LogEvent::BusDown,
        team: None,
        task: None,
        message: "dropped log".to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    });
    failing.writer.close().unwrap();
    assert_eq!(failing.history.health().lost, 0);

    let unavailable = History::unavailable(&std::io::Error::other("disk offline"), Arc::new(Reports::default()));
    unavailable.record_log(LogRecord {
        at: at(30, 0),
        module: "agent::daemon".to_owned(),
        event: LogEvent::BusDown,
        team: None,
        task: None,
        message: "dropped on unavailable".to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    });
    assert_eq!(unavailable.health().lost, 0);
}

#[test]
fn an_empty_daemon_log_module_is_rejected_by_the_api_and_sqlite() {
    let (_dir, path, mut db) = database();
    let empty_module = LogRecord {
        at: at(10, 0),
        module: String::new(),
        event: LogEvent::BusDown,
        team: None,
        task: None,
        message: "bus down".to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    };
    let api_result = db.write_logs(&[empty_module]);
    drop(db);

    let raw = Connection::open(&path).unwrap();
    let raw_result = raw.execute(
        "INSERT INTO daemon_logs (first_at, at, level, module, event, message, fields, count, cut) \
         VALUES ('2026-04-13T09:00:00Z', '2026-04-13T09:00:00Z', 'warn', '', 'bus_down', 'm', '{}', 1, '')",
        [],
    );
    assert!(
        matches!(
            api_result,
            Err(WriteError::Rule {
                record: RecordKind::Event,
                ..
            })
        ),
        "the public writer accepted an empty module: {api_result:?}"
    );
    assert!(raw_result.is_err(), "SQLite accepted an empty module");
}

#[test]
fn warning_coalescing_uses_min_and_max_timestamps_for_seeded_permutations() {
    for seed in 0..20_u64 {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let mut timestamps = [10_i64, 30, 20, 5, 25];
        for index in (1..timestamps.len()).rev() {
            let other = rng.random_range(0..=index);
            timestamps.swap(index, other);
        }
        let (_dir, path, mut db) = database();
        for timestamp in timestamps {
            db.write_logs(&[LogRecord {
                at: at(timestamp, 0),
                module: "agent::daemon".to_owned(),
                event: LogEvent::BusDown,
                team: Some(LogTeamScope::new(team_a(), [round_a(1)])),
                task: Some(LogTaskScope::new(task_a(), Some(run_a(1)))),
                message: "same warning".to_owned(),
                fields: BTreeMap::from([("operation".to_owned(), json!("inbox"))]),
                cut: Cut::default(),
            }])
            .unwrap();
        }
        let entries = Reader::open(&path).unwrap().daemon_logs().unwrap();
        assert_eq!(entries.len(), 1, "seed={seed}, order={timestamps:?}");
        assert_eq!(entries[0].count().get(), 5, "seed={seed}");
        assert_eq!(entries[0].first_at(), at(5, 0), "seed={seed}");
        assert_eq!(entries[0].at(), at(30, 0), "seed={seed}");
    }
}

#[test]
fn warning_coalescing_lookup_uses_its_equivalence_index_at_scale() {
    let (_dir, path, mut db) = database();
    for number in 0..1_000_i64 {
        db.write_logs(&[LogRecord {
            at: at(number, 0),
            module: "agent::daemon".to_owned(),
            event: LogEvent::BusDown,
            team: None,
            task: None,
            message: format!("distinct warning {number}"),
            fields: BTreeMap::new(),
            cut: Cut::default(),
        }])
        .unwrap();
    }
    drop(db);

    let raw = Connection::open(&path).unwrap();
    let plan: String = raw
        .query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM daemon_logs \
             WHERE level = 'warn' AND event = 'bus_down' AND team IS NULL AND task IS NULL AND run IS NULL \
               AND message = 'not present' AND fields = '{}' AND cut = '' ORDER BY id DESC",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(
        plan.contains("daemon_logs_warn_coalescing"),
        "warning lookup does not use its equivalence index: {plan}"
    );
}

#[test]
fn a_history_of_version_ten_keeps_its_log_and_takes_every_event_of_the_daemon_once_it_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let before: Vec<_> = MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 10)
        .collect();
    let mut old = HistoryDb::open_with(&path, &before, roomy()).unwrap();
    let record = |seconds: i64, event: LogEvent, message: &str| LogRecord {
        at: at(seconds, 0),
        module: "agent::daemon".to_owned(),
        event,
        team: None,
        task: None,
        message: message.to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    };
    old.write_logs(&[record(1, LogEvent::BusDown, "kept from version ten — ünïcode")])
        .unwrap();
    drop(old);

    let mut opened = HistoryDb::open(&path, roomy()).unwrap();
    let every: Vec<LogRecord> = LogEvent::ALL
        .iter()
        .zip(2_i64..)
        .map(|(event, seconds)| record(seconds, *event, event.as_str()))
        .collect();
    opened.write_logs(&every).unwrap();
    drop(opened);

    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let mut events: Vec<(LogEvent, String)> = logs
        .iter()
        .map(|entry| (entry.event(), entry.message().to_owned()))
        .collect();
    events.sort_by(|left, right| left.1.cmp(&right.1));
    let mut expected: Vec<(LogEvent, String)> = LogEvent::ALL
        .iter()
        .map(|event| (*event, event.as_str().to_owned()))
        .chain([(LogEvent::BusDown, "kept from version ten — ünïcode".to_owned())])
        .collect();
    expected.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(events, expected);
}
