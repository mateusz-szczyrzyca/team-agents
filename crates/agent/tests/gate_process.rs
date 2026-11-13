//! The `agent` binary as a process, the way an agent uses `agent gate`: a daemon in a temporary `AGENTS_HOME` on a
//! scratch repository with a stand-in for the `claude` CLI, two tasks, and `agent gate` run from inside their worktrees
//! at the same time. The check of the item "Gate service": the second gate waits, then runs, and both results are
//! recorded with their commit.

#[cfg(test)]
mod gate_process {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

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
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-agent-gate");
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

    /// Runs git for the setup of a test and returns its output.
    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// A home with a scratch repository `example`, two agents, three investigators for the reviews, and a daemon.
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        order: PathBuf,
        daemon: Option<Child>,
    }

    impl World {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let order = root.join("order");
            let mut config = String::new();
            for (name, table) in [
                ("inv-opus", "investigators"),
                ("inv-codex", "investigators"),
                ("inv-gemini", "investigators"),
                ("claude-1", "agents"),
                ("claude-2", "agents"),
            ] {
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"sleep\", \"--as\", \"{name}\", \
                     \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
                .unwrap();
            }
            write!(
                config,
                "[tiers]\nnormal = [\"claude-1\", \"claude-2\"]\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"{}\", \"{}\"]\n",
                script("team-cli.sh").display(),
                agentbus_binary().display(),
                script("slow-gate.sh").display(),
                order.display(),
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
                order,
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

        /// Runs `agent` with `args` in `dir`.
        fn agent_in(&self, dir: &Path, args: &[&str]) -> Command {
            let mut command = agent();
            command.args(args).current_dir(dir).env("AGENTS_HOME", &self.home);
            command
        }

        /// Starts a solo task of `agent_name` and returns its ID and its worktree, once its run is over and the files
        /// the stand-in left in the worktree are gone, so that the tree is clean.
        fn solo(&self, agent_name: &str) -> (String, PathBuf) {
            let output = self
                .agent_in(&self.repo, &["run", agent_name, "Fix the typo."])
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", stderr(&output));
            let text = stdout(&output);
            let line = text.lines().nth(1).unwrap();
            let task = line.split_whitespace().next().unwrap().to_owned();
            let worktree = PathBuf::from(line.split_whitespace().last().unwrap());
            eventually("the stand-in to run", || worktree.join(".fake-runs").is_file());
            eventually("the task to wait", || {
                let status = self.agent_in(&self.repo, &["status"]).output().unwrap();
                stdout(&status)
                    .lines()
                    .any(|line| line.contains(&task) && line.contains("waiting"))
            });
            git(&worktree, &["clean", "-fdq"]);
            (task, worktree)
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
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8(output.stdout.clone()).unwrap()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8(output.stderr.clone()).unwrap()
    }

    #[test]
    fn two_agent_gate_calls_at_once_run_one_after_the_other_and_both_are_recorded_with_their_commit() {
        let mut world = World::new();
        world.start();
        let (first, first_tree) = world.solo("claude-1");
        let (second, second_tree) = world.solo("claude-2");

        // The two agents run `agent gate` in their worktrees at the same moment.
        let mut asking = [&first_tree, &second_tree].map(|tree| {
            world
                .agent_in(tree, &["gate"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        });
        let outputs = asking.each_mut().map(|child| {
            let mut out = Vec::new();
            let mut err = Vec::new();
            std::io::Read::read_to_end(child.stdout.as_mut().unwrap(), &mut out).unwrap();
            std::io::Read::read_to_end(child.stderr.as_mut().unwrap(), &mut err).unwrap();
            (
                child.wait().unwrap(),
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        });

        for ((status, out, err), tree) in outputs.iter().zip([&first_tree, &second_tree]) {
            let head = git(tree, &["rev-parse", "HEAD"]);
            assert!(status.success(), "{err}");
            assert!(out.contains(&format!("gate: passed at {}", &head[..7])), "{out}");
        }
        let order = std::fs::read_to_string(&world.order).unwrap();
        let lines: Vec<&str> = order.lines().collect();
        assert_eq!(lines.len(), 4, "{order}");
        let (a, b) = (
            lines[0].strip_prefix("start ").unwrap(),
            lines[2].strip_prefix("start ").unwrap(),
        );
        assert_ne!(a, b);
        assert_eq!(
            lines[1],
            format!("end {a}"),
            "the second gate started before the first ended: {order}"
        );
        assert_eq!(lines[3], format!("end {b}"), "{order}");
        for (task, tree) in [(&first, &first_tree), (&second, &second_tree)] {
            let record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(world.home.join("tasks").join(task).join("gate.json")).unwrap())
                    .unwrap();
            assert_eq!(
                record["result"]["head"],
                serde_json::json!(git(tree, &["rev-parse", "HEAD"]))
            );
            assert_eq!(record["tree"], serde_json::json!("clean"));
            assert_eq!(record["result"]["outcome"], serde_json::json!({"kind": "passed"}));
        }
    }
}
