//! A scope without `--follow` prints its records and the steps of its runs as one text of the newest lines.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::process::{Command, Output};

    use agent_core::cli::ActivityEvent;
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunChanges, RunStart, RunWhy, Step, TeamStart};
    use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::log::{LogEvent, LogRecord, LogTaskScope};
    use agent_core::state::Timestamp;
    use agent_runtime::history::{HistoryDb, Store};

    /// When `second` is, after 12:50:00 UTC.
    fn at(second: u32) -> Timestamp {
        format!("2026-11-21T12:50:{second:02}Z")
            .parse()
            .expect("a literal time is valid")
    }

    /// The task numbered `number`.
    fn task(number: &str) -> TaskId {
        format!("task-20261121-132909-{number:0>4}")
            .parse()
            .expect("a literal id is valid")
    }

    /// The team of the history.
    fn team() -> TeamId {
        "team-20261121-132909-4qem".parse().expect("a literal id is valid")
    }

    /// The round of the history.
    fn round() -> RoundId {
        "round-20261121-132909-q1w2".parse().expect("a literal id is valid")
    }

    /// The start of a run of `agent`, on `cli`, which works on `task` in the round.
    fn start(agent: &str, cli: CliKind, number: &str, second: u32) -> RunStart {
        RunStart {
            run: format!("run-20261121-132909-{number:0>4}")
                .parse::<RunId>()
                .expect("a literal id is valid"),
            team: team(),
            task: task(number),
            at: at(second),
            agent: AgentIdentity {
                name: agent.to_owned(),
                tier: Tier::Normal,
                cli,
                model: None,
            },
            role: Role::Solo,
            rounds: vec![round()],
            why: RunWhy::FirstRun,
            prompt: String::new(),
            worktree: "/home/example/worktrees/example/task"
                .parse()
                .expect("a literal path is valid"),
            branch: "agents/example/task".parse().expect("a literal branch is valid"),
            session: None,
        }
    }

    /// A run's record with its steps at `seconds` and what it changed, so that the round is told.
    fn run(agent: &str, cli: CliKind, number: &str, steps: &[(u32, &str)]) -> Vec<Record> {
        let start = start(agent, cli, number, 0);
        let mut records = vec![Record::Run(Box::new(start.clone()))];
        for (number, text) in steps {
            records.push(Record::Step(Box::new(Step {
                run: start.run.clone(),
                number: NonZeroU32::new(*number).expect("a step number starts at one"),
                at: at(2 + *number),
                event: ActivityEvent::Command((*text).to_owned()),
            })));
        }
        records.push(Record::Changes(Box::new(RunChanges {
            run: start.run.clone(),
            head_before: None,
            head_after: None,
            commits: Vec::new(),
            files: Vec::new(),
            uncommitted: Vec::new(),
        })));
        records
    }

    /// A status row of `task` at `second`.
    fn row(number: &str, second: u32) -> LogRecord {
        LogRecord {
            at: at(second),
            module: "test".to_owned(),
            event: LogEvent::TaskStatusChanged,
            team: Some(agent_core::log::LogTeamScope::new(team(), [round()])),
            task: Some(LogTaskScope::new(task(number), None)),
            message: "task status changed".to_owned(),
            fields: [
                ("from".to_owned(), serde_json::json!("waiting")),
                ("to".to_owned(), serde_json::json!("queued")),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
            cut: agent_core::history::Cut::default(),
        }
    }

    /// A history of two runs of one task, of two agents, and a status row between them.
    fn history(path: &Path) -> HistoryDb {
        let mut db = HistoryDb::open(path, MaxTextBytes::default()).expect("a temporary history opens");
        let mut records = vec![Record::Team(Box::new(TeamStart {
            team: team(),
            at: at(0),
            repo: "example".to_owned(),
            base_branch: "main".parse().expect("a literal branch is valid"),
            merge: agent_core::state::MergeMode::NoMerge,
            task_list: String::new(),
            plan: Vec::new(),
        }))];
        records.extend(run(
            "inv-claude-sonnet",
            CliKind::Claude,
            "0001",
            &[(1, "cargo test"), (2, "git push")],
        ));
        db.write(&records).expect("the first run is written");
        db.write_logs(&[row("0001", 6)]).expect("the row is written");
        let second = run("codex-1", CliKind::Codex, "0002", &[(1, "cargo build")]);
        db.write(&second).expect("the second run is written");
        db
    }

    /// Runs `agent log` with `args` in `home`.
    fn log(home: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .env_clear()
            .env("AGENTS_HOME", home)
            .args(["log"])
            .args(args)
            .output()
            .expect("agent log runs")
    }

    /// The lines of a successful `agent log`.
    fn lines(home: &Path, args: &[&str]) -> Vec<String> {
        let output = log(home, args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("the text is UTF-8")
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    /// The text of `lines`, as one string.
    fn text(lines: &[String]) -> String {
        lines.join("\n")
    }

    #[test]
    fn a_task_without_follow_prints_its_records_and_the_steps_of_its_runs_in_order() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        history(&dir.path().join("history.db"));

        let found = lines(dir.path(), &["--task", "task-20261121-132909-0001"]);

        let expected = [
            "2026-11-21T12:50:00Z run run-20261121-132909-0001 started by inv-claude-sonnet on task task-20261121-132909-0001 of team team-20261121-132909-4qem (solo) in round round-20261121-132909-q1w2",
            "2026-11-21T12:50:03Z command: cargo test",
            "2026-11-21T12:50:04Z command: git push",
            "2026-11-21T12:50:06Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: queued ← waiting",
        ];
        assert_eq!(found.len(), expected.len(), "{}", text(&found));
        for (line, expected) in found.iter().zip(expected) {
            assert_eq!(line, expected);
        }
    }

    #[test]
    fn a_run_of_another_agent_prints_its_own_steps_with_its_name() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        history(&dir.path().join("history.db"));

        let found = lines(dir.path(), &["--run", "run-20261121-132909-0002"]);

        assert_eq!(
            text(&found),
            "2026-11-21T12:50:00Z run run-20261121-132909-0002 started by codex-1 on task task-20261121-132909-0002 of team team-20261121-132909-4qem (solo) in round round-20261121-132909-q1w2\n2026-11-21T12:50:03Z command: cargo build"
        );
    }

    #[test]
    fn no_steps_prints_the_records_alone() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        history(&dir.path().join("history.db"));

        let found = lines(dir.path(), &["--task", "task-20261121-132909-0001", "--no-steps"]);

        assert_eq!(found.len(), 1, "{}", text(&found));
        assert!(
            found[0].starts_with("2026-11-21T12:50:06Z info task_status_changed"),
            "{}",
            text(&found)
        );
    }

    #[test]
    fn an_agent_prints_the_steps_of_every_run_of_its_own_with_the_task_of_the_run() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        history(&dir.path().join("history.db"));

        let found = lines(dir.path(), &["--agent", "codex-1"]);

        assert_eq!(
            text(&found),
            "2026-11-21T12:50:00Z run run-20261121-132909-0002 started by codex-1 on task task-20261121-132909-0002 of team team-20261121-132909-4qem (solo) in round round-20261121-132909-q1w2\n2026-11-21T12:50:03Z command: cargo build"
        );
    }

    #[test]
    fn json_prints_the_rows_alone_as_before() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        history(&dir.path().join("history.db"));

        let found = lines(dir.path(), &["--task", "task-20261121-132909-0001", "--json"]);

        assert_eq!(found.len(), 1, "{}", text(&found));
        let value: serde_json::Value = serde_json::from_str(&found[0]).expect("a JSON line");
        assert_eq!(value["event"], "task_status_changed");
        assert_eq!(value["message"], "task status changed");
        assert_eq!(value["task"], "task-20261121-132909-0001");
    }
}
