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
                "[agents.claude-1]\ncli = \"claude\"\n[plan]\nbounces = 0\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{fake}\"\n\
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
        // The versions of the command come first, the daemon being the same build.
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            String::from_utf8(status.stdout).unwrap(),
            format!(
                "agent {version} · agent-core {version} · agent-runtime {version} · agent-web {version}\nno teams\n"
            )
        );
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
        // The child pid is written with `echo $! > .fake-child`, which creates the file before the pid is in it:
        // wait until it reads as a pid, not until it exists.
        eventually("the run to write its child pid", || {
            match std::fs::read_to_string(&marker) {
                Ok(text) if text.trim().is_empty() => false,
                Ok(text) => {
                    text.trim().parse::<i32>().unwrap();
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => panic!("cannot read child marker {}: {error}", marker.display()),
            }
        });
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

    /// Runs `agent` with `args` in `home` under `tz` and returns its standard output.
    fn run_in_zone(home: &Home, args: &[&str], tz: &str) -> String {
        let output = agent()
            .args(args)
            .env("AGENTS_HOME", home.root())
            .env("TZ", tz)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// Runs `agent log --task TASK` in `zone` until the daemon has logged `needle`: the run ends after the
    /// questions notification appears, so a single read could see the log before its end is recorded.
    fn log_until(home: &Home, zone: &str, needle: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let text = run_in_zone(home, &["log", "--task", TASK], zone);
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle} in the task log"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Seeds a team with an open round `round` on item 1.1 in progress for `TASK`, whose run asks a question.
    fn seed_open_round_questions(home: &Home, round: &str) {
        use std::num::NonZeroU32;

        use agent_core::ids::RoundId;

        let task = TaskId::from_str(TASK).unwrap();
        let repo = RepoName::from_str("team-agents").unwrap();
        let member = Member::new(task.clone(), Participant::Normal("claude-1".parse().unwrap()));
        let mut team = Team::new(
            TeamId::from_str("team-20261111-153000-t5m8").unwrap(),
            repo.clone(),
            Timestamp::from_str("2026-11-11T19:15:00Z").unwrap(),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        );
        team.open_round(
            RoundId::from_str(round).unwrap(),
            Some("1.1".parse().unwrap()),
            Timestamp::from_str("2026-11-11T19:15:00Z").unwrap(),
        )
        .unwrap();
        team.advance_item(
            "1.1".parse().unwrap(),
            agent_core::state::ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task-20261111-153001-k3x9-1.1".parse().unwrap(),
            },
        )
        .unwrap();
        let mut state = State::new();
        state.insert_team(team).unwrap();
        std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
        let worktree = home.worktree_dir(&repo, &task);
        std::fs::create_dir_all(&worktree).unwrap();
        let questions = home.questions_file(&task, NonZeroU32::MIN);
        std::fs::write(
            worktree.join(".fake-scenario"),
            format!("questions {}", questions.display()),
        )
        .unwrap();
    }

    #[test]
    fn status_round_log_and_notification_show_the_same_zone_while_status_json_stays_utc() {
        const CENTRAL_EUROPE: &str = "CET-1CEST,M3.5.0,M10.5.0/3";
        const NEW_YORK: &str = "EST5EDT,M3.2.0,M11.1.0";
        const ROUND: &str = "round-20261111-153000-q1w2";

        let (_dir, home) = home();
        seed_open_round_questions(&home, ROUND);
        let daemon = agent()
            .arg("daemon")
            .arg("--prompts-dir")
            .arg(prompts())
            .env("AGENTS_HOME", home.root())
            .env("TZ", CENTRAL_EUROPE)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let notices_dir = home.notifications_dir();
        eventually("the questions notification", || {
            std::fs::read_dir(&notices_dir).is_ok_and(|mut entries| entries.any(|entry| entry.is_ok()))
        });

        let status_east = run_in_zone(&home, &["status"], CENTRAL_EUROPE);
        let status_west = run_in_zone(&home, &["status"], NEW_YORK);
        let round_east = run_in_zone(&home, &["status", ROUND], CENTRAL_EUROPE);
        let json_east = run_in_zone(&home, &["status", "--json"], CENTRAL_EUROPE);
        let json_utc = run_in_zone(&home, &["status", "--json"], "UTC0");
        let log_east = log_until(&home, CENTRAL_EUROPE, " info run_ended");
        let notice_path = std::fs::read_dir(&notices_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "md"))
            .unwrap();
        let notice = std::fs::read_to_string(notice_path).unwrap();

        terminate(&daemon);
        let ended = finish(daemon);
        assert!(ended.status.success(), "{ended:?}");

        assert!(status_east.contains("started 2026-11-11 20:15 +01:00"), "{status_east}");
        assert!(status_west.contains("started 2026-11-11 14:15 -05:00"), "{status_west}");
        assert!(
            round_east.contains("  opened 2026-11-11 20:15:00 +01:00\n"),
            "{round_east}"
        );
        assert_eq!(json_east, json_utc);
        assert!(json_east.contains("\"2026-11-11T19:15:00Z\""), "{json_east}");

        let ended_stamp = round_east
            .lines()
            .find_map(|line| line.split_once("  ended ").map(|(_, stamp)| stamp.trim()))
            .expect("the round detail shows the ended run");
        assert!(
            log_east.contains(&format!("{ended_stamp} info run_ended")),
            "log={log_east}\nround={round_east}"
        );
        let (date_time, zone) = ended_stamp.rsplit_once(' ').expect("stamp has zone");
        let minute = date_time.get(..16).expect("minute prefix");
        assert!(
            notice.contains(&format!("\nTime: {minute} {zone}\n")),
            "notice={notice}\nended_stamp={ended_stamp}"
        );
    }

    #[test]
    fn agent_debug_uses_the_process_zone_and_status_json_is_byte_for_byte_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        const CENTRAL_EUROPE: &str = "CET-1CEST,M3.5.0,M10.5.0/3";
        const ROUND: &str = "round-20261111-153000-q1w2";

        let (dir, home) = home();
        let secrets = home.root().join("secrets.toml");
        std::fs::write(&secrets, "").unwrap();
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).unwrap();
        seed_open_round_questions(&home, ROUND);
        let daemon = start_daemon(&home);
        let socket = home.socket_file();
        eventually("the socket", || socket.exists());
        eventually("the task waiting on questions", || {
            std::fs::read_to_string(home.state_file())
                .ok()
                .and_then(|text| State::from_json(&text).ok())
                .and_then(|state| {
                    state
                        .find_task(&TaskId::from_str(TASK).unwrap())
                        .map(|found| found.member.status())
                })
                == Some(TaskStatus::Questions)
        });

        let json_east = run_in_zone(&home, &["status", "--json"], CENTRAL_EUROPE);
        let debug_out = run_in_zone(
            &home,
            &["debug", "--task", TASK, "--out", dir.path().to_str().unwrap()],
            CENTRAL_EUROPE,
        );

        terminate(&daemon);
        let ended = finish(daemon);
        assert!(ended.status.success(), "{ended:?}");

        assert_eq!(
            json_east,
            "[{\"id\":\"team-20261111-153000-t5m8\",\"repo\":\"team-agents\",\"short_desc\":\"Build it.\",\"created_at\":\"2026-11-11T19:15:00Z\",\"merge\":\"autopilot\",\"items\":{\"pending\":0,\"in_progress\":1,\"in_review\":0,\"approved\":0,\"done\":0},\"rounds\":[{\"id\":\"round-20261111-153000-q1w2\",\"item\":\"1.1\",\"opened_at\":\"2026-11-11T19:15:00Z\",\"state\":{\"status\":\"open\"}}],\"members\":[{\"task\":\"task-20261111-153001-k3x9\",\"agent\":{\"tier\":\"normal\",\"name\":\"claude-1\"},\"role\":\"solo\",\"status\":\"questions\",\"open_question\":1}]}]\n"
        );

        let summary_path = debug_out
            .lines()
            .find_map(|line| line.strip_prefix("Debug summary: "))
            .expect("debug prints the summary path");
        let summary = std::fs::read_to_string(summary_path).unwrap();
        assert!(summary.contains(" +01:00\n"), "{summary}");
        let collected = summary
            .lines()
            .find(|line| line.starts_with("Collected at: "))
            .expect("summary has a Collected at line");
        assert!(collected.ends_with(" +01:00"), "{collected}");
    }
}
