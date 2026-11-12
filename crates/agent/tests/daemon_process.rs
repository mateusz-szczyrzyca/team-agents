//! The `agent` binary as a process: `agent daemon` and `agent status` in a temporary `AGENTS_HOME`, a second daemon
//! refused, and a SIGTERM that ends the daemon and its runs with no process of theirs left.

#[cfg(test)]
mod daemon_process {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    use agent_core::config::{AbsolutePath, RepoName};
    use agent_core::ids::{TaskId, TeamId};
    use agent_core::state::{
        Crew, Member, MergeMode, Participant, RunEnd, RunState, State, TaskListText, Team, Timestamp,
    };
    use agent_core::status::TaskStatus;
    use agent_runtime::store::Home;
    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(30);

    const TASK: &str = "task-20261111-153001-k3x9";

    const LIST: &str = "Build it.\n\n1. Feature\n[ ] 1.1 Item\n    Do it.\n";

    fn agent() -> Command {
        Command::new(env!("CARGO_BIN_EXE_agent"))
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn prompts() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    /// A home with a configuration whose `claude` is the stand-in, and the lines of `extra`.
    fn home() -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(dir.path()));
        let fake = format!(
            "{}/../agent-runtime/tests/fixtures/run/fake-claude.sh",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::write(
            home.config_file(),
            format!(
                "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{fake}\"\n\
                 [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n"
            ),
        )
        .unwrap();
        (dir, home)
    }

    /// A state with one queued solo task, and its worktree running `scenario`.
    fn seed(home: &Home, scenario: &str) {
        let task = TaskId::from_str(TASK).unwrap();
        let repo = RepoName::from_str("team-agents").unwrap();
        let member = Member::new(task.clone(), Participant::Normal("claude-1".parse().unwrap()));
        let team = Team::new(
            TeamId::from_str("team-20261111-153000-t5m8").unwrap(),
            repo.clone(),
            Timestamp::from_str("2026-11-11T19:15:00Z").unwrap(),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        );
        let mut state = State::new();
        state.insert_team(team).unwrap();
        std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
        let worktree = home.worktree_dir(&repo, &task);
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".fake-scenario"), scenario).unwrap();
    }

    fn start_daemon(home: &Home) -> Child {
        agent()
            .arg("daemon")
            .arg("--prompts-dir")
            .arg(prompts())
            .env("AGENTS_HOME", home.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(child: &Child) {
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        rustix::process::kill_process(pid, Signal::TERM).unwrap();
    }

    /// Waits for the daemon to end and returns what it printed.
    fn finish(mut child: Child) -> Output {
        let deadline = Instant::now() + PATIENCE;
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the daemon did not end after SIGTERM");
            std::thread::sleep(Duration::from_millis(10));
        }
        child.wait_with_output().unwrap()
    }

    fn is_gone(pid: i32) -> bool {
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return true;
        };
        text.rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_none_or(|state| matches!(state, 'Z' | 'X'))
    }

    #[test]
    fn the_daemon_starts_status_answers_and_a_second_daemon_is_refused() {
        let (_dir, home) = home();
        let daemon = start_daemon(&home);
        let socket = home.socket_file();
        eventually("the socket", || socket.exists());

        let status = agent().arg("status").env("AGENTS_HOME", home.root()).output().unwrap();
        let second = agent()
            .arg("daemon")
            .arg("--prompts-dir")
            .arg(prompts())
            .env("AGENTS_HOME", home.root())
            .output()
            .unwrap();
        terminate(&daemon);
        let ended = finish(daemon);

        assert!(status.status.success(), "{status:?}");
        assert_eq!(String::from_utf8(status.stdout).unwrap(), "no teams\n");
        assert_eq!(second.status.code(), Some(1), "{second:?}");
        assert!(String::from_utf8(second.stderr).unwrap().contains("already running"));
        assert!(ended.status.success(), "{ended:?}");
        assert!(!socket.exists(), "the socket is removed when the daemon ends");
    }

    #[test]
    fn a_home_without_a_configuration_is_refused_by_the_daemon_with_the_file_named() {
        let dir = tempfile::tempdir().unwrap();

        let output = agent()
            .arg("daemon")
            .arg("--prompts-dir")
            .arg(prompts())
            .env("AGENTS_HOME", dir.path())
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("agents.toml"), "{stderr}");
    }

    #[test]
    fn sigterm_ends_the_daemon_and_its_runs_and_leaves_no_process_of_theirs() {
        let (_dir, home) = home();
        seed(&home, "hang");
        let daemon = start_daemon(&home);
        let marker = home
            .worktree_dir(
                &RepoName::from_str("team-agents").unwrap(),
                &TaskId::from_str(TASK).unwrap(),
            )
            .join(".fake-child");
        eventually("the run to start its child", || marker.is_file());
        let child: i32 = std::fs::read_to_string(&marker).unwrap().trim().parse().unwrap();

        terminate(&daemon);
        let ended = finish(daemon);

        assert!(ended.status.success(), "{ended:?}");
        eventually("the child of the run to be gone", || is_gone(child));
        let state = State::from_json(&std::fs::read_to_string(home.state_file()).unwrap()).unwrap();
        let found = state.find_task(&TaskId::from_str(TASK).unwrap()).unwrap();
        assert_eq!(found.member.status(), TaskStatus::Queued, "the next start resumes it");
        assert!(matches!(
            found.member.runs()[0].state(),
            RunState::Ended {
                end: RunEnd::NoStatus { exit_code: None },
                ..
            }
        ));
        let stderr = String::from_utf8(ended.stderr).unwrap();
        assert!(stderr.contains("run started"), "the daemon logs on stderr: {stderr}");
    }
}
