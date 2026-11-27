//! The environment of the run of an agent with an account, through the `agent` binary: two agents of one CLI with two
//! accounts in one team each see their own account's variables and secrets and not the other's, and an agent without an
//! account sees what it saw before accounts. A temporary `AGENTS_HOME` with the stand-in for `claude` of
//! `tests/fixtures/team-cli.sh` and the real `agentbus`.

#[cfg(test)]
mod account_process {
    use std::os::unix::fs::PermissionsExt as _;
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

    /// The variables the stand-in writes down: one set by an account's `env`, one by its secrets, one by `[agent_env]`.
    const SEEN: &str = "env-sleep:ACCOUNT_VARIABLE+ACCOUNT_SECRET+AGENT_VARIABLE";

    /// The secrets of the home: a variable of every run and a secret of each of the two accounts.
    const SECRETS: &str = "[agent_env]\nAGENT_VARIABLE = \"for-every-run\"\n\
         [account_env.comp-claude]\nACCOUNT_SECRET = \"secret-of-comp\"\n\
         [account_env.mateusz-claude]\nACCOUNT_SECRET = \"secret-of-mateusz\"\n";

    /// The accounts, and the agents that use them: `(name, account)`, the account `None` for an agent without one.
    const AGENTS: [(&str, Option<&str>); 3] = [
        ("comp-claude-1", Some("comp-claude")),
        ("mateusz-claude-1", Some("mateusz-claude")),
        ("plain-claude-1", None),
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

    /// A home with a scratch repository `example` (its origin is `https://example.com/org/example.git`), the two
    /// accounts, their agents and three investigators, and a daemon that is started when [`World::start`] is called.
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
            let mut config = String::from(
                "[accounts.comp-claude]\nowner = \"comp\"\n[accounts.comp-claude.env]\nACCOUNT_VARIABLE = \"env-of-comp\"\n\
                 [accounts.mateusz-claude]\nowner = \"mateusz\"\n[accounts.mateusz-claude.env]\nACCOUNT_VARIABLE = \"env-of-mateusz\"\n",
            );
            let arguments = |name: &str| {
                format!(
                    "args = [\"--scenario\", \"{SEEN}\", \"--as\", \"{name}\", \"--bus-bin\", \"{bus}\", \"--bus-dir\", \
                     \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
            };
            for (name, account) in AGENTS {
                config.push_str(&format!("[agents.{name}]\ncli = \"claude\"\n{}", arguments(name)));
                if let Some(account) = account {
                    config.push_str(&format!("account = \"{account}\"\n"));
                }
            }
            for name in ["inv-opus", "inv-codex", "inv-gemini"] {
                config.push_str(&format!(
                    "[investigators.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"sleep\", \"--as\", \"{name}\", \
                     \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                ));
            }
            config.push_str(&format!(
                "[plan]\nbounces = 0\n[tiers]\nnormal = [\"comp-claude-1\", \"mateusz-claude-1\", \"plain-claude-1\"]\n\
                 [binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n",
                script("team-cli.sh").display(),
                agentbus_binary().display(),
            ));
            std::fs::write(home.join("agents.toml"), config).unwrap();
            std::fs::write(home.join("secrets.toml"), SECRETS).unwrap();
            std::fs::set_permissions(home.join("secrets.toml"), std::fs::Permissions::from_mode(0o600)).unwrap();
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

        /// Runs `agent` with `args` in the scratch repository.
        fn agent(&self, args: &[&str]) -> Output {
            agent()
                .args(args)
                .current_dir(&self.repo)
                .env("AGENTS_HOME", &self.home)
                .output()
                .unwrap()
        }

        /// What the stand-in of `agent` wrote down of its environment, once it wrote all of `lines` lines.
        fn seen(&self, agent: &str, lines: usize) -> String {
            let file = self.home.join(format!("env-seen-{agent}.txt"));
            let mut text = String::new();
            eventually(&format!("the environment of {agent}"), || {
                text = std::fs::read_to_string(&file).unwrap_or_default();
                text.lines().count() == lines
            });
            text
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

    // Rule: two agents of one CLI with two accounts in one team each see the variables and secrets of their own account,
    // and none of the other's.
    #[test]
    fn two_accounts_same_model_one_team() {
        let mut world = World::new();
        world.start();
        let list = world.repo.join("list.md");
        std::fs::write(&list, LIST).unwrap();

        let output = world.agent(&[
            "run",
            "comp-claude-1",
            "inv-opus",
            "mateusz-claude-1",
            "-f",
            list.to_str().unwrap(),
        ]);

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            world.seen("comp-claude-1", 3),
            "ACCOUNT_VARIABLE=env-of-comp\nACCOUNT_SECRET=secret-of-comp\nAGENT_VARIABLE=for-every-run\n"
        );
        assert_eq!(
            world.seen("mateusz-claude-1", 3),
            "ACCOUNT_VARIABLE=env-of-mateusz\nACCOUNT_SECRET=secret-of-mateusz\nAGENT_VARIABLE=for-every-run\n"
        );
    }

    // Rule: an agent without an account gets the environment it got before accounts: `[agent_env]`, none of an account's.
    #[test]
    fn agent_without_account_sees_todays_environment() {
        let mut world = World::new();
        world.start();

        let output = world.agent(&["run", "plain-claude-1", "Fix the typo in the README."]);

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            world.seen("plain-claude-1", 3),
            "ACCOUNT_VARIABLE=<unset>\nACCOUNT_SECRET=<unset>\nAGENT_VARIABLE=for-every-run\n"
        );
    }
}
