//! `agent renew` as a process, against a daemon whose agent is a stand-in: the next run of the task starts, in a new session.

#[cfg(test)]
mod renew_process {
    use std::path::Path;
    use std::process::{Child, Command, Output, Stdio};
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    use agent_core::config::{AbsolutePath, RepoName};
    use agent_core::ids::{TaskId, TeamId};
    use agent_core::state::{Crew, Member, MergeMode, Participant, State, TaskListText, Team, Timestamp};
    use agent_runtime::store::Home;
    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(30);

    /// The task of the test.
    const TASK: &str = "task-20261111-153001-k3x9";

    /// The task list of the team.
    const LIST: &str = "Build it.\n\n1. Feature\n[ ] 1.1 Item\n    Do it.\n";

    fn agent() -> Command {
        Command::new(env!("CARGO_BIN_EXE_agent"))
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn task() -> TaskId {
        TaskId::from_str(TASK).unwrap()
    }

    fn repo() -> RepoName {
        RepoName::from_str("team-agents").unwrap()
    }

    /// A home whose `claude` is the stand-in, with a queued solo task whose worktree runs `scenario`.
    fn seeded(scenario: &str) -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(dir.path()));
        let fake = format!(
            "{}/../agent-runtime/tests/fixtures/run/fake-claude.sh",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::write(
            home.config_file(),
            format!(
                "[agents.claude-1]\ncli = \"claude\"\n[plan]\nbounces = 0\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{fake}\"\n\
                 [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n"
            ),
        )
        .unwrap();
        let member = Member::new(task(), Participant::Normal("claude-1".parse().unwrap()));
        let team = Team::new(
            TeamId::from_str("team-20261111-153000-t5m8").unwrap(),
            repo(),
            Timestamp::from_str("2026-11-11T19:15:00Z").unwrap(),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        );
        let mut state = State::new();
        state.insert_team(team).unwrap();
        std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
        let worktree = home.worktree_dir(&repo(), &task());
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".fake-scenario"), scenario).unwrap();
        (dir, home)
    }

    fn scenario(home: &Home, text: &str) {
        std::fs::write(home.worktree_dir(&repo(), &task()).join(".fake-scenario"), text).unwrap();
    }

    fn start_daemon(home: &Home) -> Child {
        let child = agent()
            .arg("daemon")
            .arg("--prompts-dir")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"))
            .env("AGENTS_HOME", home.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        eventually("the socket", || home.socket_file().exists());
        child
    }

    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs `agent` with `args` against `home`.
    fn run(home: &Home, args: &[&str]) -> Output {
        agent().args(args).env("AGENTS_HOME", home.root()).output().unwrap()
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    /// What `agent status` says of the task.
    fn status(home: &Home) -> String {
        text(run(home, &["status", TASK]).stdout)
    }

    /// The status of the task, as the line of `agent status` that names it ends with it, and what follows it.
    fn task_line(home: &Home) -> String {
        status(home)
            .lines()
            .find(|line| line.contains(TASK))
            .unwrap_or_default()
            .to_owned()
    }

    fn stop_daemon(daemon: Child) {
        let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
        rustix::process::kill_process(pid, Signal::TERM).unwrap();
        let output = daemon.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn renew_prints_its_success_line_and_the_next_run_of_the_task_starts() {
        let (_dir, home) = seeded("busy");
        let daemon = start_daemon(&home);
        eventually("the task to run", || task_line(&home).contains("  running"));
        let stopped = run(&home, &["stop", TASK]);
        assert!(stopped.status.success(), "{stopped:?}");
        eventually("the task to stop", || task_line(&home).ends_with("  stopped"));
        scenario(&home, "done");

        let renewed = run(&home, &["renew", TASK]);

        assert!(renewed.status.success(), "{renewed:?}");
        assert_eq!(
            text(renewed.stdout),
            format!("Renewed {TASK}: its next run starts a new session\n")
        );
        eventually("the task to be done", || task_line(&home).ends_with("  done"));
        stop_daemon(daemon);
    }
}
