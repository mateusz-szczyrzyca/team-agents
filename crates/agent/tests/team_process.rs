//! The `agent` binary as a process, the way a person uses it: `agent daemon`, `agent run` and `agent status` in a
//! temporary `AGENTS_HOME`, on a scratch repository with an `origin`, with a stand-in for the `claude` CLI and the real
//! `agentbus`. The checks of the item "Starting work and the team's life": a solo task and a team started, a worker
//! or too many investigators refused before anything starts, and a member that is done leading to the coordinator's
//! integration, shown by `agent status`.

#[cfg(test)]
mod team_process {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// A team of two features, one item each, for two members.
    const LIST: &str =
        "Build the things.\n\n1. First\n[ ] 1.1 One\n    Do one.\n\n2. Second\n[ ] 2.1 Two\n    Do two.\n";

    fn agent() -> Command {
        Command::new(env!("CARGO_BIN_EXE_agent"))
    }

    fn prompts() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    fn script(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// The `agentbus` built from `agentbus/`, once for all the tests of this file.
    fn agentbus_binary() -> &'static Path {
        static BINARY: OnceLock<PathBuf> = OnceLock::new();
        BINARY.get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus");
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-agent-process");
            let built = Command::new("go")
                .arg("build")
                .arg("-o")
                .arg(&binary)
                .arg(".")
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "go build of {source:?} failed: {}",
                String::from_utf8_lossy(&built.stderr)
            );
            binary
        })
    }

    /// Runs git for the setup of a test.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A home with a scratch repository `example` (its origin is `https://example.com/org/example.git`), the
    /// configuration of the agents given, and a daemon that is started when [`World::start`] is called.
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        daemon: Option<Child>,
    }

    /// `(name, kind, scenario)` of an agent: kind `agent`, `worker` or `investigator`.
    type AgentSpec = (&'static str, &'static str, &'static str);

    impl World {
        fn new(agents: &[AgentSpec]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            let (mut normals, mut workers) = (Vec::new(), Vec::new());
            for (name, kind, scenario) in agents {
                let table = if *kind == "investigator" {
                    "investigators"
                } else {
                    "agents"
                };
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \
                     \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
                .unwrap();
                match *kind {
                    "agent" => normals.push(format!("\"{name}\"")),
                    "worker" => workers.push(format!("\"{name}\"")),
                    _ => {}
                }
            }
            write!(
                config,
                "[tiers]\nnormal = [{}]\nworkers = [{}]\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
                normals.join(", "),
                workers.join(", "),
                script("team-cli.sh").display(),
                agentbus_binary().display(),
            )
            .unwrap();
            std::fs::write(home.join("agents.toml"), config).unwrap();
            let repo = root.join("example");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            git(
                &repo,
                &["remote", "add", "origin", "https://example.com/org/example.git"],
            );
            std::fs::write(repo.join("README.md"), "Hello\n").unwrap();
            git(&repo, &["add", "README.md"]);
            git(&repo, &["commit", "-q", "-m", "First"]);
            Self {
                _dir: dir,
                home,
                repo,
                daemon: None,
            }
        }

        fn start(&mut self) {
            let daemon = agent()
                .arg("daemon")
                .arg("--prompts-dir")
                .arg(prompts())
                .env("AGENTS_HOME", &self.home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let socket = self.home.join("agent.sock");
            eventually("the socket", || socket.exists());
            self.daemon = Some(daemon);
        }

        /// Runs `agent` with `args` in the scratch repository.
        fn agent(&self, args: &[&str]) -> Output {
            agent()
                .args(args)
                .current_dir(&self.repo)
                .env("AGENTS_HOME", &self.home)
                .output()
                .unwrap()
        }

        fn status(&self) -> String {
            let output = self.agent(&["status"]);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap()
        }

        /// Waits until `agent status` says what `check` wants.
        fn status_where(&self, what: &str, check: impl Fn(&str) -> bool) -> String {
            let deadline = Instant::now() + PATIENCE;
            loop {
                let status = self.status();
                if check(&status) {
                    return status;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}:\n{status}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        /// The runs the task has had, as its stand-in counted them.
        fn runs(&self, task: &str) -> u32 {
            std::fs::read_to_string(self.home.join("worktrees/example").join(task).join(".fake-runs"))
                .map_or(0, |text| text.trim().parse().unwrap())
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            if let Some(mut daemon) = self.daemon.take() {
                let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
                rustix::process::kill_process(pid, Signal::TERM).unwrap();
                daemon.wait().unwrap();
            }
        }
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8(output.stdout.clone()).unwrap()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8(output.stderr.clone()).unwrap()
    }

    const REVIEWERS: [AgentSpec; 3] = [
        ("inv-opus", "investigator", "sleep"),
        ("inv-codex", "investigator", "sleep"),
        ("inv-gemini", "investigator", "sleep"),
    ];

    fn agents_with(extra: &[AgentSpec]) -> Vec<AgentSpec> {
        REVIEWERS.iter().chain(extra).copied().collect()
    }

    #[test]
    fn agent_run_with_one_agent_and_a_text_starts_a_solo_task_which_runs_to_done() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "done")]));
        world.start();

        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);

        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        assert!(text.starts_with("team team-"), "{text}");
        assert!(text.contains("started on example, merge: autopilot"), "{text}");
        assert!(text.contains("  solo  claude-1  "), "{text}");
        let task = text
            .lines()
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();
        assert!(
            Path::new(text.lines().nth(1).unwrap().split_whitespace().last().unwrap()).is_dir(),
            "{text}"
        );
        let status = world.status_where("the task to be done", |status| {
            status.contains(&format!("{task}  solo")) && status.contains("done")
        });
        assert!(status.contains("claude-1"), "{status}");
    }

    #[test]
    fn agent_run_with_two_agents_and_a_file_starts_a_team_led_by_the_investigator() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "sleep"),
            ("claude-2", "agent", "sleep"),
        ]));
        world.start();
        let list = world.repo.join("list.md");
        std::fs::write(&list, LIST).unwrap();

        let output = world.agent(&["run", "claude-1", "inv-opus", "claude-2", "-f", list.to_str().unwrap()]);

        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[1].contains("coordinator  inv-opus"), "{text}");
        assert!(
            lines[2].contains("member       claude-1") && lines[2].ends_with("item 1.1"),
            "{text}"
        );
        assert!(
            lines[3].contains("member       claude-2") && lines[3].ends_with("item 2.1"),
            "{text}"
        );
        let status = world.status_where("every task to wait", |status| status.matches("waiting").count() == 3);
        assert!(status.contains("0 pending, 2 in progress"), "{status}");
    }

    #[test]
    fn a_worker_as_coordinator_and_too_many_investigators_are_refused_before_anything_starts() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "sleep"),
            ("glm-1", "worker", "sleep"),
        ]));
        world.start();

        let worker = world.agent(&["run", "claude-1", "glm-1", "--coordinator", "glm-1", "Do it."]);
        let crowded = world.agent(&["run", "inv-opus", "inv-codex", "claude-1", "Do it."]);

        assert!(!worker.status.success());
        assert!(
            stderr(&worker).contains("workers serve delegate requests and never coordinate"),
            "{}",
            stderr(&worker)
        );
        assert!(!crowded.status.success());
        assert!(
            stderr(&crowded).contains("too many investigators in the team"),
            "{}",
            stderr(&crowded)
        );
        assert_eq!(world.status(), "no teams\n");
        assert!(
            !world.home.join("worktrees").exists()
                || std::fs::read_dir(world.home.join("worktrees/example"))
                    .map_or(true, |mut entries| entries.next().is_none())
        );
    }

    #[test]
    fn a_member_that_is_done_wakes_the_coordinator_which_agent_status_shows() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "slowdone"),
            ("claude-2", "agent", "sleep"),
        ]));
        world.start();

        let output = world.agent(&["run", "inv-opus", "claude-1", "claude-2", "Do it all."]);
        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        let coordinator = text
            .lines()
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();

        eventually("the coordinator to be woken for the member that is done", || {
            world.runs(&coordinator) >= 2
        });
        let status = world.status_where("every task to wait", |status| status.matches("waiting").count() == 3);

        assert!(status.contains("  coordinator  inv-opus  waiting"), "{status}");
    }
}
