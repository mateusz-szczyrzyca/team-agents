//! Queries against a daemon-created history of two teams and three completed rounds.
use super::history_stories_tests::{log, patient};
use super::rig::{CHAIN, ONE, Rig, investigator, normal};
use agent_core::config::Secrets;
use agent_core::history::Record;
use agent_core::log::LogLevel;
use agent_core::state::{MergeMode, Timestamp};
use agent_runtime::history::{HistoryLayer, Reader};
use serde_json::Value;
use std::collections::BTreeMap;

/// Decodes the public JSON-lines interface, not database internals.
fn rows(rig: &Rig, args: &[&str]) -> Vec<Value> {
    log(rig, args)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn two_teams_three_rounds_prove_every_filter_pair_view_and_literal_search() {
    let mut rig = patient(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r4", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
    let marked = CHAIN.replace("Build the thing.", "Zażółć gęślą AND OR NEAR % * \"cytat\"");
    let (first, _) = rig.run(&["claude-1"], &marked, MergeMode::NoMerge, None).await;
    let (second, _) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    rig.wait_for("three closed rounds", || {
        let reader = Reader::open(&rig.home.history_file()).unwrap();
        [&first, &second].iter().map(|team| reader.team_records(team).unwrap().iter().filter(|record| {
            matches!(&record.record, Record::Event(event) if matches!(event.event, agent_core::history::HistoryEvent::RoundClosed { .. }))
        }).count()).sum::<usize>() == 3
    }).await;
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let all = rows(&rig, &["log", "--json", "-n", "100000"]);
    assert!(!all.is_empty());
    let mut agents = BTreeMap::new();
    for team in [&first, &second] {
        for run in reader.runs_of_team(team).unwrap() {
            agents.insert(run.start.value.run.to_string(), run.start.value.agent.name);
        }
    }
    assert_filters(&rig, &all, &agents);
    for (flag, scope) in [("--team", first.to_string()), ("--team", second.to_string())]
        .into_iter()
        .chain([&first, &second].into_iter().flat_map(|team| {
            reader
                .rounds_of_team(team)
                .unwrap()
                .into_iter()
                .map(|round| ("--round", round.value.round.to_string()))
        }))
    {
        assert_views(&rig, &reader, flag, &scope);
    }
    for text in ["Zażółć", "Zażółć gęślą AND OR NEAR % * \"cytat\"", "fine", "run"] {
        let found = rows(&rig, &["log", "--grep", text, "--json", "-n", "100000"]);
        assert!(!found.is_empty(), "search {text:?}");
    }
    assert!(rows(&rig, &["log", "--grep", "absentsearchmarker", "--json"]).is_empty());
    assert!(rows(&rig, &["log", "-n", "0", "--json"]).is_empty());
    assert_eq!(rows(&rig, &["log", "-n", "1", "--json"]), all[all.len() - 1..]);
}

/// Computes each single predicate and every conjunction independently of SQL.
fn assert_filters(rig: &Rig, all: &[Value], agents: &BTreeMap<String, String>) {
    let anchor = all
        .iter()
        .find(|row| row["run"].is_string() && row["task"].is_string() && !row["rounds"].as_array().unwrap().is_empty())
        .unwrap();
    let run = anchor["run"].as_str().unwrap();
    let filters = [
        ("--team", anchor["team"].as_str().unwrap()),
        ("--round", anchor["rounds"][0].as_str().unwrap()),
        ("--task", anchor["task"].as_str().unwrap()),
        ("--run", run),
        ("--agent", agents[run].as_str()),
        ("--level", anchor["level"].as_str().unwrap()),
        ("--event", anchor["event"].as_str().unwrap()),
        ("--since", anchor["at"].as_str().unwrap()),
        ("--until", anchor["at"].as_str().unwrap()),
    ];
    for (index, filter) in filters.iter().enumerate() {
        for other in std::iter::once(None).chain(filters.iter().skip(index + 1).map(Some)) {
            let selected: Vec<_> = std::iter::once(filter).chain(other).collect();
            let mut args = vec!["log", "--json", "-n", "100000"];
            for (flag, value) in &selected {
                args.extend([*flag, *value]);
            }
            if selected.iter().any(|(flag, _)| matches!(*flag, "--team" | "--round")) {
                args.extend(["--show", "log"]);
            }
            let expected: Vec<_> = all
                .iter()
                .filter(|row| {
                    selected.iter().all(|(flag, value)| match *flag {
                        "--round" => row["rounds"].as_array().unwrap().iter().any(|r| r == value),
                        "--agent" => row["run"]
                            .as_str()
                            .and_then(|run| agents.get(run))
                            .is_some_and(|agent| agent == value),
                        "--since" => {
                            row["at"].as_str().unwrap().parse::<Timestamp>().unwrap() >= value.parse().unwrap()
                        }
                        "--until" => {
                            row["at"].as_str().unwrap().parse::<Timestamp>().unwrap() <= value.parse().unwrap()
                        }
                        flag => row[flag.strip_prefix("--").unwrap()].as_str() == Some(value),
                    })
                })
                .cloned()
                .collect();
            assert!(!expected.is_empty(), "{selected:?}");
            assert_eq!(rows(rig, &args), expected, "{selected:?}");
        }
    }
}

/// Checks that projections contain exactly the scoped runs, including their status and final head.
fn assert_views(rig: &Rig, reader: &Reader, flag: &str, scope: &str) {
    let runs = if flag == "--team" {
        reader.runs_of_team(&scope.parse().unwrap()).unwrap()
    } else {
        reader.runs_of_round(&scope.parse().unwrap()).unwrap()
    };
    for view in [
        "tasks",
        "agents",
        "worktrees",
        "runs",
        "prompts",
        "changes",
        "reviews",
        "log",
        "usage",
    ] {
        let values = rows(rig, &["log", flag, scope, "--show", view, "--json"]);
        assert!(!values.is_empty(), "{scope}: {view}");
        if view == "prompts" {
            assert_eq!(values.len(), runs.len());
            for run in &runs {
                let found = values
                    .iter()
                    .find(|row| row["run"] == run.start.value.run.as_str())
                    .unwrap();
                assert_eq!(found["prompt"], run.start.value.prompt);
            }
        }
        if view == "agents" {
            assert!(
                values
                    .iter()
                    .all(|row| row["role"].is_string() && row["how"].is_string())
            );
        }
        if view == "tasks" {
            assert!(values.iter().all(|row| row["status"] == "DONE"), "{values:?}");
        }
    }
}
