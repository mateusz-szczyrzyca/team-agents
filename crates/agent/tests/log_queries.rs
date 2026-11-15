//! Direct-history query contracts, independent of a running daemon.
#[cfg(test)]
#[path = "support/story_history.rs"]
mod story_history;

use std::process::Command;

#[test]
fn scoped_views_and_literal_search_select_only_their_subject() {
    let dir = tempfile::tempdir().unwrap();
    let written = story_history::write_history(&dir.path().join("history.db"));
    for (flag, scope) in [
        ("--team", written.team.as_str()),
        ("--round", written.rounds[0].as_str()),
    ] {
        for view in [
            "tasks",
            "agents",
            "worktrees",
            "runs",
            "prompts",
            "changes",
            "reviews",
            "usage",
        ] {
            let result = Command::new(env!("CARGO_BIN_EXE_agent"))
                .env("AGENTS_HOME", dir.path())
                .args(["log", flag, scope, "--show", view, "--json"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{flag} {view}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let lines: Vec<serde_json::Value> = String::from_utf8(result.stdout)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert!(!lines.is_empty(), "{flag} {view}");
            if view == "usage" {
                assert_eq!(lines[0]["message"], "Tokens are not recorded yet (11.1).");
            }
            if view == "agents" {
                for row in lines {
                    assert!(row.get("role").is_some());
                    assert!(row.get("how").is_some());
                }
            }
        }
    }
}

#[test]
fn grep_is_a_literal_log_filter_and_stats_work_without_a_daemon() {
    use agent_core::history::{Cut, MaxTextBytes};
    use agent_core::log::{LogEvent, LogRecord};
    use agent_runtime::history::{HistoryDb, Store};
    let dir = tempfile::tempdir().unwrap();
    let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).unwrap();
    db.write_logs(&[LogRecord {
        at: "2026-11-15T00:00:00Z".parse().unwrap(),
        module: "test".to_owned(),
        event: LogEvent::TeamChanged,
        team: None,
        task: None,
        message: "Zażółć AND OR NEAR \"quoted\" % *".to_owned(),
        fields: std::collections::BTreeMap::default(),
        cut: Cut::default(),
    }])
    .unwrap();
    for (text, count) in [
        ("Zażółć", 1),
        ("AND OR NEAR", 1),
        ("\"quoted\" % *", 1),
        ("absent", 0),
        ("% *", 0),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_agent"))
            .env("AGENTS_HOME", dir.path())
            .args(["log", "--grep", text, "--json"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{text}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().lines().count(),
            count,
            "{text}"
        );
    }
    let result = Command::new(env!("CARGO_BIN_EXE_agent"))
        .env("AGENTS_HOME", dir.path())
        .args(["log", "--stats", "--json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["tables"]["daemon_logs"], 1);
    assert!(stats["bytes"].as_u64().unwrap() > 0);
    assert_eq!(stats["oldest"], "2026-11-15T00:00:00.000000000Z");
}

#[test]
fn statistics_read_a_legacy_database_without_migrating_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let written = story_history::write_old_history(&path);
    let before = agent_runtime::history::Reader::open(&path).unwrap().schema().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent"))
        .env("AGENTS_HOME", dir.path())
        .args(["log", "--stats", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stats: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stats["tables"]["teams"], 1);
    let reader = agent_runtime::history::Reader::open(&path).unwrap();
    assert_eq!(reader.schema().unwrap(), before);
    assert!(reader.team(&written.team).unwrap().is_some());
}

#[test]
fn statistics_include_the_oldest_record_before_the_ledger_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let written = story_history::write_old_history(&path);
    let reader = agent_runtime::history::Reader::open(&path).unwrap();
    let oldest = reader.team(&written.team).unwrap().unwrap().value.at;
    let stats = reader.statistics().unwrap();
    assert_eq!(stats["oldest"], oldest.to_string());
}
