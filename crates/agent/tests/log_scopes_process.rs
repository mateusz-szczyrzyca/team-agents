//! A scope without `--follow` covers the runs of an agent in every team, and a history written before the steps were
//! printed by `agent log` is read and followed as one written now.

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::process::{Command, Output, Stdio};

    use agent_core::cli::ActivityEvent;
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy, Step, TeamStart};
    use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::state::Timestamp;
    use agent_runtime::history::{HistoryDb, Store};

    /// When `second` is, after 12:50:00 UTC.
    fn at(second: u32) -> Timestamp {
        format!("2026-11-21T12:50:{second:02}Z")
            .parse()
            .expect("a literal time is valid")
    }

    /// The team with the suffix `suffix`.
    fn team(suffix: &str) -> TeamId {
        format!("team-20261121-132909-{suffix}")
            .parse()
            .expect("a literal id is valid")
    }

    /// The start of run `number` of `agent` on its own task in the team `suffix`.
    fn start(number: u32, agent: &str, suffix: &str, second: u32) -> RunStart {
        RunStart {
            run: format!("run-20261121-132909-{number:04}")
                .parse::<RunId>()
                .expect("a literal id is valid"),
            team: team(suffix),
            task: format!("task-20261121-132909-{number:04}")
                .parse::<TaskId>()
                .expect("a literal id is valid"),
            at: at(second),
            agent: AgentIdentity {
                name: agent.to_owned(),
                tier: Tier::Normal,
                cli: CliKind::Claude,
                model: None,
            },
            role: Role::Solo,
            rounds: vec![
                format!("round-20261121-132909-{suffix}")
                    .parse::<RoundId>()
                    .expect("a literal id is valid"),
            ],
            why: RunWhy::FirstRun,
            prompt: String::new(),
            worktree: "/home/example/worktrees/example/task"
                .parse()
                .expect("a literal path is valid"),
            branch: "agents/example/task".parse().expect("a literal branch is valid"),
            session: None,
        }
    }

    /// A command step of run `number`, its first, at `second`.
    fn step(number: u32, second: u32, text: &str) -> Record {
        Record::Step(Box::new(Step {
            run: format!("run-20261121-132909-{number:04}")
                .parse()
                .expect("a literal id is valid"),
            number: NonZeroU32::MIN,
            at: at(second),
            event: ActivityEvent::Command(text.to_owned()),
        }))
    }

    /// Runs `agent log` with `args` in `home`.
    fn log(home: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .env_clear()
            .env("TZ", "CET-1CEST,M3.5.0,M10.5.0/3")
            .env("AGENTS_HOME", home)
            .arg("log")
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

    #[test]
    fn an_agent_prints_the_steps_of_its_runs_in_two_teams_each_with_its_team() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).expect("a history opens");
        for suffix in ["aaaa", "bbbb"] {
            db.write(&[Record::Team(Box::new(TeamStart {
                team: team(suffix),
                at: at(0),
                repo: "example".to_owned(),
                base_branch: "main".parse().expect("a literal branch is valid"),
                merge: agent_core::state::MergeMode::NoMerge,
                task_list: String::new(),
                plan: Vec::new(),
            }))])
            .expect("the team is written");
        }
        db.write(&[
            Record::Run(Box::new(start(1, "claude-1", "aaaa", 1))),
            step(1, 2, "cargo build"),
            Record::Run(Box::new(start(2, "claude-1", "bbbb", 3))),
            step(2, 4, "cargo test"),
            Record::Run(Box::new(start(3, "codex-1", "bbbb", 5))),
            step(3, 6, "not of claude-1"),
        ])
        .expect("the runs are written");

        let found = lines(dir.path(), &["--agent", "claude-1"]);

        assert_eq!(found.len(), 4, "{found:?}");
        assert!(
        found[0].contains(" run run-20261121-132909-0001 started by claude-1 on task task-20261121-132909-0001 of team team-20261121-132909-aaaa "),
        "{found:?}"
    );
        assert_eq!(found[1], "2026-11-21 13:50:02 +01:00 command: cargo build");
        assert!(found[2].contains("of team team-20261121-132909-bbbb "), "{found:?}");
        assert_eq!(found[3], "2026-11-21 13:50:04 +01:00 command: cargo test");
    }

    /// A history of the schema version 13, as an older daemon wrote it, with one run of `codex-1` and its step.
    fn old_history(path: &Path) {
        let raw = rusqlite::Connection::open(path).expect("a database opens");
        raw.execute_batch(include_str!(
            "../../agent-runtime/tests/history/fixtures/history-v13-before-delivery-close.sql"
        ))
        .expect("the old schema is created");
        raw.execute_batch(
        "INSERT INTO teams VALUES ('team-20261121-132909-aaaa', '2026-11-21T12:50:00Z', 'example', 'main', 'no_merge', '', '');
         INSERT INTO runs (run, team, task, at, agent, tier, cli, model, role, why, prompt, worktree, branch, cut)
           VALUES ('run-20261121-132909-0001', 'team-20261121-132909-aaaa', 'task-20261121-132909-0001',
                   '2026-11-21T12:50:01Z', 'codex-1', 'normal', 'codex', NULL, 'solo', 'first_run', '',
                   '/home/example/worktrees/example/task', 'agents/example/task', '');
         INSERT INTO run_rounds VALUES ('run-20261121-132909-0001', 1, 'round-20261121-132909-aaaa');
         INSERT INTO steps (run, n, at, kind, text, cut) VALUES
           ('run-20261121-132909-0001', 1, '2026-11-21T12:50:02Z', 'command', 'cargo build', '');",
    )
    .expect("the old rows are written");
    }

    #[test]
    fn a_history_of_an_older_schema_is_told_with_its_steps() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        old_history(&dir.path().join("history.db"));

        let found = lines(dir.path(), &["--agent", "codex-1"]);

        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].contains(" run run-20261121-132909-0001 started by codex-1 on task task-20261121-132909-0001 "),
            "{found:?}"
        );
        assert_eq!(found[1], "2026-11-21 13:50:02 +01:00 command: cargo build");
    }

    #[test]
    fn a_history_of_an_older_schema_is_followed_from_its_newest_lines() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        old_history(&dir.path().join("history.db"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent"))
            .env_clear()
            .env("TZ", "CET-1CEST,M3.5.0,M10.5.0/3")
            .env("AGENTS_HOME", dir.path())
            .args(["log", "--follow", "--task", "task-20261121-132909-0001", "-n", "5"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("agent log follows");
        let stdout = child.stdout.take().expect("the following has output");
        let mut reader = BufReader::new(stdout);
        let mut first = String::new();
        let mut second = String::new();
        reader.read_line(&mut first).expect("the run is told");
        reader.read_line(&mut second).expect("the step is told");
        child.kill().expect("the following stops");
        child.wait().expect("the following is reaped");

        assert!(
            first.contains(" run run-20261121-132909-0001 started by codex-1 "),
            "{first}"
        );
        assert_eq!(second, "2026-11-21 13:50:02 +01:00 command: cargo build\n");
    }
}
