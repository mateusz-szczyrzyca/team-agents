//! The `agent` binary as processes, the way a person steers it: a task that asks and is answered with `agent answer
//! --file`, and a running task that is paused, unpaused and stopped with `agent pause`, `unpause` and `stop`.

#[cfg(test)]
mod steering_process {
    use std::num::NonZeroU32;
    use std::path::{Path, PathBuf};
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

    const TASK: &str = "task-20261111-153001-k3x9";

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
                "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{fake}\"\n\
                 [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n"
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

    fn questions_file(home: &Home) -> PathBuf {
        home.questions_file(&task(), NonZeroU32::new(1).unwrap())
    }

    #[test]
    fn a_task_that_asks_leaves_its_file_and_a_notification_and_goes_on_with_the_answers_of_agent_answer_file() {
        let (_dir, home) = seeded("busy");
        scenario(&home, &format!("questions {}", questions_file(&home).display()));
        let daemon = start_daemon(&home);

        eventually("the questions to be asked", || {
            status(&home).contains("questions number 1 wait for answers")
        });

        let file = std::fs::read_to_string(questions_file(&home)).unwrap();
        assert!(
            file.contains(&format!("agent answer {TASK}")),
            "the instructions: {file}"
        );
        let notes: Vec<String> = std::fs::read_dir(home.notifications_dir())
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains(TASK) && notes[0].contains("questions-1.md"),
            "{notes:?}"
        );

        let answers = home.root().join("answers.md");
        std::fs::write(&answers, file.replace("**Verdict:**\n", "**Verdict:** ok\n")).unwrap();
        scenario(&home, "done");
        let answered = run(&home, &["answer", TASK, "--file", answers.to_str().unwrap()]);
        assert!(answered.status.success(), "{answered:?}");
        assert!(text(answered.stdout).contains("Answered questions number 1"));
        eventually("the task to be done", || task_line(&home).ends_with("  done"));

        let args = std::fs::read_to_string(home.worktree_dir(&repo(), &task()).join(".fake-args")).unwrap();
        assert!(
            args.contains("the human accepts the recommendation: a) — simpler (fake)"),
            "{args}"
        );
        stop_daemon(daemon);
    }

    #[test]
    fn pause_unpause_and_stop_change_the_status_and_the_activity_names_the_client() {
        let (_dir, home) = seeded("busy");
        let daemon = start_daemon(&home);
        eventually("the task to run", || task_line(&home).ends_with("  running"));

        let paused = run(&home, &["pause", TASK]);
        assert!(paused.status.success(), "{paused:?}");
        assert_eq!(text(paused.stdout), format!("Paused {TASK}\n"));
        assert!(task_line(&home).ends_with("  paused"), "{}", status(&home));

        let unpaused = run(&home, &["unpause", TASK]);
        assert!(unpaused.status.success(), "{unpaused:?}");
        eventually("the task to run again", || task_line(&home).ends_with("  running"));

        let stopped = run(&home, &["stop", TASK]);
        assert!(stopped.status.success(), "{stopped:?}");
        assert!(task_line(&home).ends_with("  stopped"), "{}", status(&home));

        let again = run(&home, &["stop", TASK]);
        assert_eq!(again.status.code(), Some(1));
        assert!(text(again.stderr).contains("already stopped"));
        let log = std::fs::read_to_string(home.activity_log(&task())).unwrap();
        let controls: Vec<serde_json::Value> = log
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|line| line["kind"] == "control")
            .collect();
        let actions: Vec<&str> = controls.iter().map(|line| line["action"].as_str().unwrap()).collect();
        assert_eq!(actions, ["pause", "unpause", "stop"]);
        assert!(
            controls
                .iter()
                .all(|line| line["by"] == serde_json::json!({"kind": "cli"})),
            "{controls:?}"
        );
        stop_daemon(daemon);
    }
}
