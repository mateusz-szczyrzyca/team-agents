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
use std::fmt::Write;

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
    assert_ne!(all, Vec::<Value>::new());
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
    assert_eq!(
        rows(&rig, &["log", "--grep", "absentsearchmarker", "--json"]),
        Vec::<Value>::new()
    );
    assert_eq!(rows(&rig, &["log", "-n", "0", "--json"]), Vec::<Value>::new());
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

#[tokio::test]
async fn daemon_daily_retention_removes_closed_round_and_gate_but_keeps_old_open_round() {
    use super::history_stories_tests::gate_when_committed;
    let mut rig = patient(
        &[
            normal("claude-1", "gated"),
            normal("claude-2", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[history]\nkeep_days = 1\n[log]\nkeep_days = 1\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
    let (closed_team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    gate_when_committed(&rig, &tasks[0], 1).await;
    let (open_team, _) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    rig.wait_for("closed first round", || {
        Reader::open(&rig.home.history_file()).unwrap().team_records(&closed_team).unwrap().iter().any(|r| matches!(&r.record, Record::Event(e) if matches!(e.event, agent_core::history::HistoryEvent::RoundClosed { .. })))
    }).await;
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let gate = reader
        .team_records(&closed_team)
        .unwrap()
        .into_iter()
        .find_map(|row| match row.record {
            Record::Event(e) => match e.event {
                agent_core::history::HistoryEvent::Gate { record, .. } => Some(record.result.log),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert!(gate.as_path().is_file());
    let before = reader.statistics().unwrap();
    assert_eq!(before["tables"]["rounds"], 2);
    rig.advance_history_days(2);
    rig.wait_for("daily retention", || reader.team(&closed_team).unwrap().is_none())
        .await;
    assert!(reader.team(&open_team).unwrap().is_some());
    assert_eq!(reader.rounds_of_team(&open_team).unwrap().len(), 1);
    assert!(!gate.as_path().exists());
    let after = rows(&rig, &["log", "--stats"]);
    assert_eq!(after[0]["tables"]["rounds"], 1);
    assert_eq!(reader.team_records(&closed_team).unwrap(), Vec::new());
    rig.stop().await.unwrap();
    // Startup performs the same cleanup, including with an old open round still present.
    rig.advance_history_days(2);
    rig.start().await;
    rig.stop().await.unwrap();
    assert_eq!(reader.rounds_of_team(&open_team).unwrap().len(), 1);
}

#[tokio::test]
async fn task_and_worktree_views_show_running_status_and_the_last_recorded_head() {
    let mut rig = patient(
        &[
            normal("claude-1", "gated"),
            normal("claude-2", "gated"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, task) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    super::history_stories_tests::gate_when_committed(&rig, &task[0], 1).await;
    rig.wait_for("finished changes", || {
        Reader::open(&rig.home.history_file())
            .unwrap()
            .team_records(&team)
            .unwrap()
            .iter()
            .any(|r| matches!(&r.record, Record::Changes(c) if c.head_after != c.head_before))
    })
    .await;
    let (open, _) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    rig.wait_for("open run", || {
        !Reader::open(&rig.home.history_file())
            .unwrap()
            .runs_of_team(&open)
            .unwrap()
            .is_empty()
    })
    .await;
    let tasks = rows(&rig, &["log", "--team", open.as_str(), "--show", "tasks", "--json"]);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["status"], "running");
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let records = reader.team_records(&team).unwrap();
    let changes = records
        .iter()
        .find_map(|r| match &r.record {
            Record::Changes(c) if c.head_after != c.head_before => Some(c),
            _ => None,
        })
        .unwrap();
    let run = reader.run(&changes.run).unwrap().unwrap();
    let trees = rows(&rig, &["log", "--team", team.as_str(), "--show", "worktrees", "--json"]);
    let tree = trees
        .iter()
        .find(|tree| tree["task"] == run.start.value.task.as_str())
        .unwrap();
    assert_eq!(tree["head"], serde_json::to_value(&changes.head_after).unwrap());
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn focused_views_of_a_daemon_round_have_normalized_golden_output() {
    use super::history_stories_tests::{Normalized, golden};
    let mut rig = patient(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "late:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(8707)).await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.wait_for("closed round", || Reader::open(&rig.home.history_file()).unwrap().team_records(&team).unwrap().iter().any(|r| matches!(&r.record, Record::Event(e) if matches!(e.event, agent_core::history::HistoryEvent::RoundClosed { .. })))).await;
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let round = reader.rounds_of_team(&team).unwrap()[0].value.round.clone();
    let json = log(&rig, &["log", "--team", team.as_str(), "--json"]);
    let mut text = String::new();
    for (scope, id) in [("--round", round.as_str()), ("--team", team.as_str())] {
        for view in ["tasks", "agents", "worktrees", "prompts", "usage"] {
            writeln!(text, "{scope} --show {view}").unwrap();
            text.push_str(&log(&rig, &["log", scope, id, "--show", view]));
        }
        writeln!(text, "{scope} --show log --event run_started").unwrap();
        text.push_str(&log(
            &rig,
            &["log", scope, id, "--show", "log", "--event", "run_started"],
        ));
    }
    golden("log-focused-views.txt", &Normalized::of(&rig, &text, &json));
}

#[tokio::test]
async fn daemon_recorded_questions_answers_steps_and_final_messages_are_searchable() {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    use agent_core::status::TaskStatus;

    let mut rig = patient(
        &[
            normal("claude-1", "ask,stream:search"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("search.jsonl"), concat!(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"żółwstepmarker"}]}}"#,
        "\n",
    )).unwrap();
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: std::num::NonZeroU32::MIN,
        verdict: Verdict::from_text("answersearchmarker").unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: std::num::NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    for query in [
        "What should the team do about the pull request",
        "answersearchmarker",
        "żółwstepmarker",
        "Streamed",
        "I have to ask",
    ] {
        let found = rows(&rig, &["log", "--grep", query, "--json", "-n", "100000"]);
        assert!(!found.is_empty(), "query {query:?}");
        assert!(
            found.iter().all(|row| row["task"] == tasks[0].to_string()),
            "{query:?}: {found:?}"
        );
    }
    assert_eq!(
        rows(&rig, &["log", "--grep", "unrecordedanswersearchmarker", "--json"]),
        Vec::<Value>::new()
    );
}
