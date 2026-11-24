//! A run that has Serena, the `agent` binary as a process in a temporary `AGENTS_HOME`: with `[binaries] serena` the
//! first prompt the stand-in CLI is given holds the section on finding code with Serena, and without it the prompt is
//! the one of every run before it.

#[cfg(test)]
mod serena_prompt {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// The heading of the section on finding code with Serena, as `prompts/harness/run/serena.md` opens it.
    const SECTION: &str = "## Finding code: Serena";

    /// The heading the working context opens with.
    const CONTEXT: &str = "# Working context";

    fn agent() -> Command {
        Command::new(env!("CARGO_BIN_EXE_agent"))
    }

    fn prompts() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    fn script(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
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

    /// A home with a scratch repository `example`, one agent whose runs keep their prompt in a file of the home, and a
    /// daemon that is started when [`World::start`] is called.
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        daemon: Option<Child>,
    }

    impl World {
        /// A world whose `agents.toml` names `serena` when `with_serena`. Its value is a program no run of these tests
        /// starts: the stand-in CLI calls no tool and commits nothing. The value of `serena` is a program no run of
        /// these tests starts: the stand-in only prints the call of a tool, as its stream does.
        fn new(with_serena: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            for (table, name) in [
                ("agents", "claude-1"),
                ("investigators", "inv-opus"),
                ("investigators", "inv-codex"),
                ("investigators", "inv-gemini"),
            ] {
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"claude\"\nargs = [\"--prompt-file\", \"{home}/{name}-prompt.txt\", \
                     \"--status\", \"STATUS: DONE\"]\n",
                    home = home.display(),
                )
                .unwrap();
            }
            let serena = if with_serena { "serena = \"serena\"\n" } else { "" };
            write!(
                config,
                "[plan]\nbounces = 0\n[tiers]\nnormal = [\"claude-1\"]\n\
                 [binaries]\nclaude = \"{}\"\n{serena}\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n",
                script("serena-cli.sh").display(),
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
            // Two investigators outside a team of one review its pull request at the default of `[review]`; a team of
            // two needs a third.
            Self {
                _dir: dir,
                home,
                repo,
                daemon: None,
            }
        }

        /// Starts the daemon, its standard error in `daemon.log` of the home, and waits for its socket.
        fn start(&mut self) {
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.home.join("daemon.log"))
                .unwrap();
            let daemon = agent()
                .arg("daemon")
                .arg("--prompts-dir")
                .arg(prompts())
                .env("AGENTS_HOME", &self.home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
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

        /// The first prompt the runs of `agent` were given.
        fn first_prompt(&self, agent: &str) -> String {
            let file = self.home.join(format!("{agent}-prompt.txt"));
            eventually("the first prompt of the run", || file.is_file());
            std::fs::read_to_string(file).unwrap()
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

    #[test]
    fn a_solo_run_with_serena_is_told_how_to_find_code_right_after_its_context() {
        let mut world = World::new(true);
        world.start();
        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

        let prompt = world.first_prompt("claude-1");

        let context = prompt.find(CONTEXT).expect("the working context is in the prompt");
        let section = prompt.find(SECTION).expect("the section on Serena is in the prompt");
        assert!(context < section, "{prompt}");
        let between = prompt.get(context + CONTEXT.len()..section).unwrap();
        let rest = prompt.get(section + SECTION.len()..).unwrap();
        assert!(
            between.contains("Ending your turn") && rest.contains("Your role: solo"),
            "the section follows the working context and comes before the role: {prompt}"
        );
        assert!(
            prompt.contains("`find_symbol`") && prompt.contains("`find_referencing_symbols`"),
            "{prompt}"
        );
    }

    #[test]
    fn a_reviewer_with_serena_is_told_how_to_find_code() {
        let mut world = World::new(true);
        world.start();
        let output = world.agent(&["run", "inv-opus", "Review the pull request."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

        let prompt = world.first_prompt("inv-opus");

        assert!(prompt.contains("You are agent **inv-opus**"), "{prompt}");
        assert!(prompt.contains(SECTION), "{prompt}");
    }

    #[test]
    fn a_run_without_serena_is_not_told_how_to_find_code() {
        let mut world = World::new(false);
        world.start();
        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

        let prompt = world.first_prompt("claude-1");

        assert!(prompt.contains(CONTEXT), "{prompt}");
        assert!(!prompt.contains(SECTION), "{prompt}");
        assert!(!prompt.to_ascii_lowercase().contains("serena"), "{prompt}");
    }
}
