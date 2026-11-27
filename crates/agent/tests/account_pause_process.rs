//! The pause of an account through the whole harness: `agent daemon` in a temporary `AGENTS_HOME`, the stand-in CLI
//! `team-cli.sh` for every agent and the real `agentbus`. A run of one agent of an account that its provider refuses
//! pauses every task of every agent of that account at once, until the same reset, with one notification; agents of
//! other accounts, also of another account of the same owner, run on.

#[cfg(test)]
mod account_pause_process {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// How long a task is watched to see that it does not start.
    const WATCH: Duration = Duration::from_secs(2);

    /// The account whose agents these tests pause.
    const ACCOUNT: &str = "comp-claude";

    /// The agents of the home, with the account each has (`None` for none) and the scenario its stand-in runs.
    ///
    /// `comp-hold` keeps its first task running; `comp-quota` is refused for its quota in its first run and does its work
    /// in the second; `comp-fresh` and `comp-other`, `other-1` finish at once. `comp-other` has the second account of the
    /// owner `comp`, `other-1` the account of another owner.
    const AGENTS: [(&str, Option<&str>, &str); 5] = [
        ("comp-hold", Some(ACCOUNT), "hold"),
        ("comp-quota", Some(ACCOUNT), "turn-limit-quota,done"),
        ("comp-fresh", Some(ACCOUNT), "done"),
        ("comp-other", Some("comp-second"), "done"),
        ("other-1", Some("other-claude"), "done"),
    ];

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
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-account-pause");
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
                "go build failed: {}",
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

    /// A home with a scratch repository `example`, the agents of [`AGENTS`] and a daemon that [`World::start`] starts.
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        daemon: Option<Child>,
    }

    impl World {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            for (account, owner) in [
                ("comp-claude", "comp"),
                ("comp-second", "comp"),
                ("other-claude", "other"),
            ] {
                writeln!(config, "[accounts.{account}]\nowner = \"{owner}\"").unwrap();
            }
            let mut names = Vec::new();
            for (name, account, scenario) in AGENTS {
                let account = account
                    .map(|account| format!("account = \"{account}\"\n"))
                    .unwrap_or_default();
                write!(
                    config,
                    "[agents.{name}]\ncli = \"claude\"\n{account}args = [\"--scenario\", \"{scenario}\", \"--as\", \
                     \"{name}\", \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
                .unwrap();
                names.push(format!("\"{name}\""));
            }
            for name in ["inv-opus", "inv-codex", "inv-gemini"] {
                write!(
                    config,
                    "[investigators.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"sleep\", \"--as\", \"{name}\", \
                     \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
                .unwrap();
            }
            write!(
                config,
                "[plan]\nbounces = 0\n[tiers]\nnormal = [{}]\nworkers = []\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n",
                names.join(", "),
                script("team-cli.sh").display(),
                agentbus_binary().display(),
            )
            .unwrap();
            std::fs::write(home.join("agents.toml"), config).unwrap();
            let repo = root.join("example");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            let origin = root.join("remote/example.git");
            std::fs::create_dir_all(&origin).unwrap();
            git(&origin, &["init", "-q", "--bare", "-b", "main"]);
            git(
                &repo,
                &[
                    "config",
                    &format!("url.{}.insteadOf", origin.to_str().unwrap()),
                    "https://example.com/org/example.git",
                ],
            );
            git(
                &repo,
                &["remote", "add", "origin", "https://example.com/org/example.git"],
            );
            std::fs::write(repo.join("README.md"), "Hello\n").unwrap();
            git(&repo, &["add", "README.md"]);
            git(&repo, &["commit", "-q", "-m", "First"]);
            git(&repo, &["push", "-q", "origin", "main"]);
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

        /// Stops the daemon with a TERM, as the service manager does.
        fn stop(&mut self) {
            if let Some(mut daemon) = self.daemon.take() {
                let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
                rustix::process::kill_process(pid, Signal::TERM).unwrap();
                daemon.wait().unwrap();
            }
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

        /// Starts a solo task of `agent` and returns its ID.
        fn run(&self, agent: &str) -> String {
            let output = self.agent(&["run", agent, "Its own work."]);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .to_owned()
        }

        fn status(&self) -> String {
            let output = self.agent(&["status", "--task"]);
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

        /// Lets the run of `task` end: the `hold` scenario waits for this file in the worktree of the task.
        fn release(&self, task: &str) {
            std::fs::write(
                self.home.join("worktrees/example").join(task).join(".fake-continue"),
                "",
            )
            .unwrap();
        }

        /// The texts of the notifications of the home that tell about the pause of `account`.
        fn account_notices(&self, account: &str) -> Vec<String> {
            let Ok(entries) = std::fs::read_dir(self.home.join("notifications")) else {
                return Vec::new();
            };
            entries
                .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
                .filter(|text| text.contains(&format!("Account {account} is paused")))
                .collect()
        }

        /// Every notification text of the home.
        fn notices(&self) -> String {
            let Ok(entries) = std::fs::read_dir(self.home.join("notifications")) else {
                return String::new();
            };
            entries
                .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
                .collect()
        }

        /// Changes the state file of the stopped daemon: `change` gets the JSON of the state.
        fn edit_state(&self, change: impl FnOnce(&mut serde_json::Value)) {
            let path = self.home.join("state.json");
            let mut state: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            change(&mut state);
            std::fs::write(path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        }

        /// The tasks the pause of an account holds, as the state file records them.
        fn held_tasks(&self) -> Vec<String> {
            let state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(self.home.join("state.json")).unwrap()).unwrap();
            state["account_held_tasks"]
                .as_array()
                .map(|tasks| tasks.iter().map(|task| task.as_str().unwrap().to_owned()).collect())
                .unwrap_or_default()
        }

        /// The end of the pause of `account` that the state file records.
        fn pause_until(&self, account: &str) -> String {
            let state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(self.home.join("state.json")).unwrap()).unwrap();
            state["accounts"][account]["until"].as_str().unwrap().to_owned()
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            self.stop();
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

    /// The line of `status` about `task`.
    fn line_of<'a>(status: &'a str, task: &str) -> &'a str {
        status.lines().find(|line| line.contains(task)).unwrap_or_default()
    }

    /// Checks for [`WATCH`] that `task` has no run: the daemon is given the time to start it if it would.
    fn never_started(world: &World, task: &str) {
        let deadline = Instant::now() + WATCH;
        while Instant::now() < deadline {
            assert_eq!(world.runs(task), 0, "{}", world.status());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The tasks a refusal leaves paused: the one whose run was refused and the one that waited for its agent.
    struct Paused {
        refused: String,
        waiting: String,
    }

    /// Runs a task of `comp-hold` that keeps its agent, a second one that waits for the agent, and a task of
    /// `comp-quota` whose first run the provider refuses for its quota; waits until the account is paused and lets the
    /// held run end.
    fn pause_the_account(world: &World) -> Paused {
        let holder = world.run("comp-hold");
        world.status_where("the first task of comp-hold to run", |_| world.runs(&holder) == 1);
        let waiting = world.run("comp-hold");
        let refused = world.run("comp-quota");
        let status = world.status_where(
            "the task of the refused run and the waiting one to be paused",
            |status| line_of(status, &refused).contains("paused") && line_of(status, &waiting).contains("paused"),
        );
        assert!(
            status.contains(&format!("account {ACCOUNT} paused until ")),
            "the status says that the account is paused:\n{status}"
        );
        world.release(&holder);
        Paused { refused, waiting }
    }

    #[test]
    fn exhausted_quota_pauses_the_account_once() {
        let mut world = World::new();
        world.start();

        let Paused { refused, waiting } = pause_the_account(&world);

        assert_eq!(world.runs(&waiting), 0, "{}", world.status());
        assert_eq!(world.runs(&refused), 1);
        eventually("the notification of the account", || {
            world.account_notices(ACCOUNT).len() == 1
        });
        let notice = world.account_notices(ACCOUNT).remove(0);
        for agent in ["comp-hold", "comp-quota", "comp-fresh"] {
            assert!(notice.contains(agent), "the notice names the agent {agent}: {notice}");
        }
        assert!(!notice.contains("comp-other"), "{notice}");
        let (other_owner, same_owner) = (world.run("other-1"), world.run("comp-other"));
        world.status_where("the agents of the other accounts to finish", |status| {
            line_of(status, &other_owner).contains("done") && line_of(status, &same_owner).contains("done")
        });
        assert_eq!(world.account_notices(ACCOUNT).len(), 1);
        assert!(
            !world.notices().contains("refused its run"),
            "the refused task has no notice of its own: {}",
            world.notices()
        );
    }

    #[test]
    fn a_fresh_task_of_a_paused_account_waits_for_the_reset() {
        let mut world = World::new();
        world.start();
        pause_the_account(&world);

        let fresh = world.run("comp-fresh");

        never_started(&world, &fresh);
    }

    #[test]
    fn account_resumes_at_reset() {
        let mut world = World::new();
        world.start();
        let Paused { refused, waiting } = pause_the_account(&world);
        let fresh = world.run("comp-fresh");
        world.stop();
        world.edit_state(|state| {
            state["accounts"][ACCOUNT]["until"] = "2020-01-01T00:00:00Z".into();
            state.as_object_mut().unwrap().remove("refused_agents");
        });

        world.start();

        world.status_where("every paused task to run again", |_| {
            world.runs(&refused) == 2 && world.runs(&waiting) == 1 && world.runs(&fresh) == 1
        });
        let status = world.status_where("the refused task to be done", |status| {
            line_of(status, &refused).contains("done")
        });
        assert!(!status.contains(&format!("account {ACCOUNT} paused")), "{status}");
    }

    #[test]
    fn held_task_without_the_pause_of_its_account_runs_after_a_restart() {
        let mut world = World::new();
        world.start();
        let Paused { refused, waiting } = pause_the_account(&world);
        world.stop();
        world.edit_state(|state| {
            let accounts = state["accounts"].as_object_mut().unwrap();
            accounts.remove(ACCOUNT).unwrap();
            state.as_object_mut().unwrap().remove("refused_agents");
            assert!(state["account_held_tasks"].as_array().unwrap().len() >= 2, "{state}");
        });

        world.start();

        world.status_where("the held tasks to run, their account has no pause", |_| {
            world.runs(&refused) == 2 && world.runs(&waiting) == 1
        });
    }

    #[test]
    fn a_task_the_human_stops_is_no_longer_held_by_the_account() {
        let mut world = World::new();
        world.start();
        let Paused { refused, waiting } = pause_the_account(&world);

        let stopped = world.agent(&["stop", &waiting]);
        assert!(stopped.status.success(), "{}", String::from_utf8_lossy(&stopped.stderr));

        world.status_where("the task to be stopped", |status| line_of(status, &waiting).contains("stopped"));
        let held = world.held_tasks();
        assert!(!held.contains(&waiting), "{held:?}");
        assert!(held.contains(&refused), "{held:?}");
    }

    #[test]
    fn a_task_the_human_pauses_is_no_longer_held_by_the_account() {
        let mut world = World::new();
        world.start();
        let Paused { refused, waiting } = pause_the_account(&world);

        let paused = world.agent(&["pause", &waiting]);
        assert!(paused.status.success(), "{}", String::from_utf8_lossy(&paused.stderr));

        let held = world.held_tasks();
        assert!(!held.contains(&waiting), "{held:?}");
        assert!(held.contains(&refused), "{held:?}");
    }

    #[test]
    fn restart_keeps_account_pause() {
        let mut world = World::new();
        world.start();
        let Paused { refused, waiting } = pause_the_account(&world);
        eventually("the notification of the account", || {
            world.account_notices(ACCOUNT).len() == 1
        });
        let until = world.pause_until(ACCOUNT);
        world.stop();

        world.start();

        let status = world.status_where("the account to be paused after the restart", |status| {
            status.contains(&format!("account {ACCOUNT} paused until "))
        });
        assert!(line_of(&status, &waiting).contains("paused"), "{status}");
        assert!(line_of(&status, &refused).contains("paused"), "{status}");
        never_started(&world, &waiting);
        assert_eq!(world.runs(&refused), 1, "{status}");
        assert_eq!(world.pause_until(ACCOUNT), until);
        assert_eq!(world.account_notices(ACCOUNT).len(), 1);
    }
}
