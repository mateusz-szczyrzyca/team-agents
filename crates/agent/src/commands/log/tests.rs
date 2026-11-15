//! Defaults and the explicitly specified story/row switch.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::ExitCode;

use agent_core::history::{Cut, MaxTextBytes};
use agent_core::ids::{RoundId, TeamId};
use agent_core::log::{LogEvent, LogRecord, LogTeamScope};
use agent_runtime::history::{HistoryDb, Store};

use crate::{Environment, run_in};

#[test]
fn default_rows_and_intersected_scopes_print_exact_json_and_limit_edges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let team: TeamId = "team-20261111-193001-t5m8".parse().unwrap();
    let round: RoundId = "round-20261111-193001-q1w2".parse().unwrap();
    let mut db = HistoryDb::open(&path.join("history.db"), MaxTextBytes::default()).unwrap();
    for message in ["first", "second", "third"] {
        db.write_logs(&[LogRecord {
            at: "2026-11-15T00:00:00Z".parse().unwrap(),
            module: "example".to_owned(),
            event: LogEvent::TeamChanged,
            team: Some(LogTeamScope::new(team.clone(), [round.clone()])),
            task: None,
            message: message.to_owned(),
            fields: BTreeMap::new(),
            cut: Cut::default(),
        }])
        .unwrap();
    }
    let environment = Environment::new(Some(OsString::from(&path)));
    for (args, messages) in [
        (vec!["log", "--json"], vec!["first", "second", "third"]),
        (
            vec!["log", "--json", "--round", round.as_str(), "--team", team.as_str()],
            vec!["first", "second", "third"],
        ),
        (
            vec!["log", "--json", "--round", round.as_str(), "--show", "log"],
            vec!["first", "second", "third"],
        ),
        (vec!["log", "--json", "--team", team.as_str(), "-n", "1"], vec!["third"]),
        (vec!["log", "--json", "-n", "0"], vec![]),
        (vec!["log", "--json", "--level", "warn"], vec![]),
        (vec!["log", "--json", "--until", "2026-11-14T23:59:59Z"], vec![]),
        (
            vec![
                "log",
                "--json",
                "--team",
                "team-20990101-000000-zzzz",
                "--round",
                round.as_str(),
            ],
            vec![],
        ),
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let status = run_in(
            &environment,
            std::iter::once("agent").chain(args.iter().copied()),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!((status, err), (ExitCode::SUCCESS, vec![]), "{args:?}");
        let values: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            values
                .iter()
                .map(|v| v["message"].as_str().unwrap())
                .collect::<Vec<_>>(),
            messages,
            "{args:?}"
        );
        for value in values {
            assert_eq!(value["team"], team.as_str());
            assert_eq!(value["rounds"], serde_json::json!([round]));
            assert_eq!(value["event"], "team_changed");
        }
    }
}

#[test]
fn row_argument_errors_preserve_usage_status_without_opening_history() {
    for args in [
        vec!["--task", "invalid"],
        vec!["--run", "invalid"],
        vec!["--agent", ""],
        vec!["--level", "trace"],
        vec!["--event", "unknown"],
        vec!["--since", "tomorrow"],
        vec!["--until", "invalid"],
        vec!["-n", "-1"],
        vec!["-n", "4294967296"],
        vec!["--show", "log"],
        vec!["--round", "round-20261111-193001-q1w2", "--show", "unknown"],
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let status = run_in(
            &Environment::new(None),
            ["agent", "log"].into_iter().chain(args.iter().copied()),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(status, ExitCode::from(2), "{args:?}");
        assert_eq!(out, b"");
        assert_ne!(err, b"");
    }
}
