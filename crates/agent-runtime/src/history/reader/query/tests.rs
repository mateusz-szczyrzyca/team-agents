//! Plans of the actual log SQL on a history with one hundred thousand steps.
use super::*;
use crate::history::{HistoryDb, schema::MIGRATIONS};
use agent_core::history::MaxTextBytes;

#[test]
fn every_row_filter_and_pair_uses_an_index_with_a_hundred_thousand_steps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    drop(HistoryDb::open(&path, MaxTextBytes::default()).unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("INSERT INTO runs(run,team,task,at,agent,tier,cli,role,why,prompt,worktree,branch,cut) VALUES('run-20261115-120000-aaaa','team-20261115-120000-aaaa','task-20261115-120000-aaaa','2026-11-15T12:00:00.000000000Z','claude-1','normal','claude','solo','first_run','searchneedle','/tmp/worktree','feature','')").unwrap();
    // This fixture proves query plans, not recording: recording is tested through daemon runs.
    raw.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO steps(run,n,at,kind,text,cut) SELECT 'run-20261115-120000-aaaa',x,'2026-11-15T12:00:00.000000000Z','message','searchneedle','' FROM n").unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM steps", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        100_000
    );
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        MIGRATIONS.last().unwrap().version
    );
    let filters = [
        LogFilter::default(),
        LogFilter {
            team: Some("team-20261115-120000-aaaa".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            round: Some("round-20261115-120000-aaaa".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            task: Some("task-20261115-120000-aaaa".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            run: Some("run-20261115-120000-aaaa".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            agent: Some("claude-1".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            level: Some(LogLevel::Info),
            ..LogFilter::default()
        },
        LogFilter {
            event: Some(LogEvent::RunStarted),
            ..LogFilter::default()
        },
        LogFilter {
            since: Some("2026-11-15T12:00:00Z".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            until: Some("2026-11-16T12:00:00Z".parse().unwrap()),
            ..LogFilter::default()
        },
        LogFilter {
            grep: Some("searchneedle".to_owned()),
            ..LogFilter::default()
        },
    ];
    for (index, first) in filters.iter().enumerate() {
        for second in filters.iter().skip(index) {
            let filter = LogFilter {
                team: first.team.clone().or_else(|| second.team.clone()),
                round: first.round.clone().or_else(|| second.round.clone()),
                task: first.task.clone().or_else(|| second.task.clone()),
                run: first.run.clone().or_else(|| second.run.clone()),
                agent: first.agent.clone().or_else(|| second.agent.clone()),
                level: first.level.or(second.level),
                event: first.event.or(second.event),
                since: first.since.or(second.since),
                until: first.until.or(second.until),
                grep: first.grep.clone().or_else(|| second.grep.clone()),
            };
            for window in [LogWindow::Newest(100), LogWindow::After { id: 1, limit: 100 }] {
                let (sql, values) = log_query(&filter, window);
                let plan = raw
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                    .unwrap()
                    .query_map(params_from_iter(values), |r| r.get::<_, String>(3))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert!(
                    plan.iter()
                        .any(|line| line.contains("INDEX") || line.contains("INTEGER PRIMARY KEY")),
                    "{filter:?} {window:?}: {plan:?}"
                );
                assert!(
                    !plan.iter().any(|line| line == "SCAN l"),
                    "unindexed log scan: {plan:?}"
                );
            }
        }
    }
    assert_reader_query_plans(&raw);
}

/// Checks `EXPLAIN QUERY PLAN` for every story, view and task-step query on `Reader`.
fn assert_reader_query_plans(raw: &rusqlite::Connection) {
    for (name, sql, values) in [
        (
            "team_records",
            "SELECT h.sequence, h.kind, h.key, h.at, h.team, h.payload, h.cut, r.round \
             FROM history_records AS h LEFT JOIN history_record_rounds AS r ON r.sequence = h.sequence \
             WHERE h.team = ?1 ORDER BY h.sequence, r.position",
            vec![Value::from("team-20261115-120000-aaaa".to_owned())],
        ),
        (
            "round_records",
            "SELECT h.sequence, h.kind, h.key, h.at, h.team, h.payload, h.cut, r.round \
             FROM history_records AS h LEFT JOIN history_record_rounds AS r ON r.sequence = h.sequence \
             WHERE h.sequence IN (SELECT sequence FROM history_record_rounds WHERE round = ?1) \
             OR (h.kind = ?2 AND h.team = (SELECT hr.team FROM history_records AS hr \
                 JOIN history_record_rounds AS rr ON rr.sequence = hr.sequence WHERE rr.round = ?1 LIMIT 1)) \
             ORDER BY h.sequence, r.position",
            vec![
                Value::from("round-20261115-120000-aaaa".to_owned()),
                Value::from("team".to_owned()),
            ],
        ),
        (
            "rounds_of_team",
            "SELECT round FROM rounds WHERE team = ?1 ORDER BY at, rowid",
            vec![Value::from("team-20261115-120000-aaaa".to_owned())],
        ),
        (
            "runs_of_team",
            "SELECT run FROM runs WHERE team = ?1 ORDER BY at, rowid",
            vec![Value::from("team-20261115-120000-aaaa".to_owned())],
        ),
        (
            "runs_of_round",
            "SELECT runs.run FROM run_rounds JOIN runs ON runs.run = run_rounds.run \
             WHERE run_rounds.round = ?1 ORDER BY runs.at, runs.rowid",
            vec![Value::from("round-20261115-120000-aaaa".to_owned())],
        ),
        (
            "prompts_of_round",
            "SELECT runs.run, runs.at, runs.agent, runs.role, runs.prompt, runs.cut FROM run_rounds \
             JOIN runs ON runs.run = run_rounds.run WHERE run_rounds.round = ?1 ORDER BY runs.at, runs.rowid",
            vec![Value::from("round-20261115-120000-aaaa".to_owned())],
        ),
        (
            "steps",
            "SELECT n, at, kind, text, path, tool, detail, action, client, login, cut \
             FROM steps WHERE run = ?1 ORDER BY n",
            vec![Value::from("run-20261115-120000-aaaa".to_owned())],
        ),
        (
            "steps_of_task",
            "SELECT s.n,s.at,s.kind,s.text,s.path,s.tool,s.detail,s.action,s.client,s.login,s.cut,s.run \
             FROM runs r JOIN steps s ON s.run=r.run WHERE r.task=?1 ORDER BY r.at,r.rowid,s.n",
            vec![Value::from("task-20261115-120000-aaaa".to_owned())],
        ),
    ] {
        let plan = raw
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(params_from_iter(values), |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|line| line.contains("INDEX") || line.contains("PRIMARY KEY")),
            "{name}: {plan:?}"
        );
        assert!(
            !plan.iter().any(|line| line.starts_with("SCAN ")),
            "unindexed scan in {name}: {plan:?}"
        );
    }
}
