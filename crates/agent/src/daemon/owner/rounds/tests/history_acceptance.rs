//! Remaining acceptance checks use daemon-produced rows, never hand-inserted recording fixtures.

use std::collections::BTreeMap;
use std::path::Path;

use agent_core::history::{CommitRecord, HowChosen, RoundSubject};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::{ItemPhase, MergeMode, RunState};
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;
use rusqlite::{Connection, OpenFlags, ToSql};

use super::rig::{ONE, Rig, git, investigator, normal};

/// Both outside reviewers approve every review pass.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

#[tokio::test]
async fn a_coordinator_line_is_recorded_unless_the_command_line_overrides_it() {
    for (flag, expected, how) in [
        (None, "inv-lead", HowChosen::NamedByLine),
        (Some("claude-1"), "claude-1", HowChosen::NamedByFlag),
    ] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", "sleep"),
                investigator("inv-lead", "sleep"),
                investigator("inv-r1", APPROVING),
                investigator("inv-r2", APPROVING),
            ],
            5,
            true,
        )
        .await;
        rig.start().await;
        let text = format!("Coordinator: inv-lead\n\n{ONE}");
        let (team, _) = rig
            .run(&["claude-1", "inv-lead"], &text, MergeMode::NoMerge, flag)
            .await;
        rig.stop().await.unwrap();
        let reader = Reader::open(&rig.home.history_file()).unwrap();
        let start = reader.team(&team).unwrap().unwrap().value;
        assert_eq!(start.task_list, text);
        assert_eq!(start.merge, MergeMode::NoMerge);
        assert_eq!(
            start
                .plan
                .iter()
                .map(|entry| (entry.agent.as_str(), entry.position.get()))
                .collect::<Vec<_>>(),
            [("claude-1", 1), ("inv-lead", 2)]
        );
        for entry in start.plan {
            if entry.agent.as_str() == expected {
                assert_eq!((entry.role, entry.how), (Role::Coordinator, how.clone()));
            } else {
                assert_eq!((entry.role, entry.how), (Role::Member, HowChosen::Named));
            }
        }
    }
}

#[tokio::test]
async fn a_brief_and_its_timestamps_survive_without_a_task_list_or_a_live_daemon() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "done"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let brief = "Explain the café's 🦀 layout.\nKeep the original spacing.\n";
    let (team, tasks) = rig.run(&["claude-1"], brief, MergeMode::NoMerge, None).await;
    let state = rig.status_is(&tasks[0], TaskStatus::Done).await;
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let start = reader.team(&team).unwrap().unwrap().value;
    assert_eq!(start.task_list, brief);
    assert_eq!(start.at, state.teams()[0].created_at());
    let rounds = reader.rounds_of_team(&team).unwrap();
    assert_eq!(rounds.len(), 1);
    assert_eq!(
        rounds[0].value.subject,
        RoundSubject::Brief("Explain the café's 🦀 layout.\nKeep the original spacing.".to_owned())
    );
    assert_eq!(rounds[0].value.at, state.teams()[0].rounds()[0].opened_at());
    let runs = reader.runs_of_round(&rounds[0].value.round).unwrap();
    assert_eq!(runs.len(), 1);
    let recorded = &runs[0];
    let actual = &state.find_task(&tasks[0]).unwrap().member.runs()[0];
    assert_eq!(recorded.start.value.at, actual.started_at());
    assert_eq!(recorded.start.value.agent.name, "claude-1");
    assert_eq!(recorded.start.value.agent.tier, Tier::Normal);
    assert_eq!(recorded.start.value.agent.cli, CliKind::Claude);
    assert_eq!(recorded.start.value.agent.model, None);
    let RunState::Ended { at, end } = actual.state() else {
        panic!("run did not end")
    };
    let ended = &recorded.end.as_ref().unwrap().value;
    assert_eq!((&ended.at, &ended.end), (at, end));
    assert_eq!(ended.final_message.as_deref(), Some("Finished the work.\nSTATUS: DONE"));
}

/// Execute a documented statement with only the parameters it actually names.
fn query(connection: &Connection, sql: &str, names: &[(&str, &str)]) -> Vec<Vec<String>> {
    let mut statement = connection.prepare(sql).unwrap();
    let mut bound: Vec<(&str, &dyn ToSql)> = Vec::new();
    for (name, value) in names {
        if statement.parameter_index(name).unwrap().is_some() {
            bound.push((name, value));
        }
    }
    let columns = statement.column_count();
    statement
        .query_map(bound.as_slice(), |row| {
            (0..columns)
                .map(|column| {
                    row.get::<_, rusqlite::types::Value>(column).map(|value| match value {
                        rusqlite::types::Value::Null => "NULL".to_owned(),
                        rusqlite::types::Value::Integer(number) => number.to_string(),
                        rusqlite::types::Value::Real(number) => number.to_string(),
                        rusqlite::types::Value::Text(text) => text,
                        rusqlite::types::Value::Blob(bytes) => format!("{} bytes", bytes.len()),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[tokio::test]
async fn every_documented_sql_query_reads_a_merged_round_recorded_by_the_daemon() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let run = state.find_task(&tasks[0]).unwrap().member.runs()[0].id();
    let round = &reader.rounds_of_team(&team).unwrap()[0].value.round;
    let connection = Connection::open_with_flags(rig.home.history_file(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let answers = page_answers(
        &connection,
        &[
            (":run", run.as_str()),
            (":round", round.as_str()),
            (":team", team.as_str()),
        ],
    );
    assert_eq!(answers.len(), 10);
    let recorded = reader.run(run).unwrap().unwrap();
    let runs = reader.runs_of_round(round).unwrap();
    assert_eq!(
        answers["runs of a round"]
            .iter()
            .map(|row| (row[1].clone(), row[2].clone(), row[3].clone()))
            .collect::<Vec<_>>(),
        runs.iter()
            .map(|row| (
                row.start.value.run.to_string(),
                row.start.value.agent.name.clone(),
                row.start.value.role.to_string()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        answers["the prompt of a run"],
        [vec![recorded.start.value.prompt.clone(), String::new()]]
    );
    assert_eq!(
        format!("{}\n", recorded.start.value.prompt),
        std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-prompt-1")).unwrap()
    );
    assert_eq!(answers["the prompts of a round"].len(), runs.len());
    let launch = recorded.launch.unwrap().value;
    assert_eq!(
        answers["the launch of a run"][0],
        [
            launch.program.clone(),
            serde_json::to_string(&launch.args).unwrap(),
            launch.working_dir.as_path().display().to_string(),
            launch.head.unwrap().to_string(),
            serde_json::to_string(&launch.env_names).unwrap()
        ]
    );
    let steps = reader.steps(run).unwrap();
    assert_ne!(steps.len(), 0);
    assert_eq!(
        answers["the steps of a run"]
            .iter()
            .map(|row| row[0].parse::<u32>().unwrap())
            .collect::<Vec<_>>(),
        steps.iter().map(|step| step.value.number.get()).collect::<Vec<_>>()
    );
    let changes = reader.changes(run).unwrap().unwrap().value;
    assert_eq!(
        answers["the files a run changed"].len(),
        changes.files.len() + changes.uncommitted.len()
    );
    assert_commit(
        &rig.worktree(&tasks[0]),
        &changes.commits,
        &answers["the commits of a run"],
    );
    assert_eq!(answers["the plan of a team"].len(), 1);
    assert_eq!(answers["the item of a round"].len(), 1);
    assert_eq!(answers["how the runs of a team ended"].len(), runs.len());
    assert!(
        answers["how the runs of a team ended"]
            .iter()
            .all(|row| row[2] != "NULL")
    );
}

/// Check authored time against Git itself as well as the SQL and typed readers.
fn assert_commit(worktree: &Path, commits: &[CommitRecord], answer: &[Vec<String>]) {
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    let actual = git(worktree, &["show", "-s", "--format=%H%n%s%n%at", commit.id.as_str()]);
    assert_eq!(
        actual,
        format!(
            "{}\n{}\n{}",
            commit.id,
            commit.subject,
            commit.authored_at.as_utc().unix_timestamp()
        )
    );
    let rows = answer;
    assert_eq!(rows.len(), 1);
    assert_eq!(&rows[0][..3], ["1", commit.id.as_str(), commit.subject.as_str()]);
    assert_eq!(
        rows[0][3].parse::<agent_core::state::Timestamp>().unwrap(),
        commit.authored_at
    );
}

/// Run every SQL block in the documentation against the daemon's database.
fn page_answers(connection: &Connection, names: &[(&str, &str)]) -> BTreeMap<String, Vec<Vec<String>>> {
    let page = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/history.md")).unwrap();
    page.split("```sql\n")
        .skip(1)
        .map(|section| {
            let sql = section.split_once("```").unwrap().0;
            let name = sql.lines().next().unwrap().strip_prefix("-- ").unwrap();
            (name.to_owned(), query(connection, sql, names))
        })
        .collect()
}
