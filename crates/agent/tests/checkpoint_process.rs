//! The checkpoints of the `agent` binary as a process: `agent daemon` in a temporary `AGENTS_HOME` with a stand-in for the
//! `claude` CLI. A daemon killed (`SIGKILL`) after a run made a commit and before the next look at its worktree loses no
//! commit: the daemon that starts next records it as a `daemon_started` checkpoint.

#[cfg(test)]
mod checkpoint_process {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use agent_core::history::{CheckpointCause, HistoryEvent, Record};
    use agent_runtime::history::Reader;
    use rustix::process::{Pid, Signal};

    /// The longest any wait of this test lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// The stand-in script of the run: the first run commits and holds; a run after it ends at once. It holds for at most
    /// a minute, so that no run outlives a test that failed.
    const SCRIPT: &str = "set -eu\n\
        sig=\"$SIGNALS\"\n\
        if [ -e \"$sig/first\" ]; then exit 0; fi\n\
        git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q --allow-empty -m one\n\
        touch \"$sig/first\"\n\
        for _ in $(seq 1 1200); do [ -e \"$sig/release\" ] && break; sleep 0.05; done\n";

    /// The `agent` binary.
    fn agent() -> Command {
        Command::new(env!("CARGO_BIN_EXE_agent"))
    }

    /// A file of the fixtures of the tests of this crate.
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// Runs git for a test, as a fixture author, and returns what it prints.
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
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A home with a scratch repository `example` and one agent, and a daemon started by [`World::start`].
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        signals: PathBuf,
        daemon: Option<Child>,
    }

    impl World {
        /// A home whose agent runs the script of [`SCRIPT`].
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            let signals = root.join("signals");
            std::fs::create_dir_all(home.join("fake-scripts")).unwrap();
            std::fs::create_dir(&signals).unwrap();
            std::fs::write(
                home.join("fake-scripts/commits.sh"),
                SCRIPT.replace("$SIGNALS", &signals.display().to_string()),
            )
            .unwrap();
            let mut config = String::new();
            for (table, name, scenario) in [
                ("agents", "claude-1", "script:commits"),
                ("investigators", "inv-opus", "sleep"),
                ("investigators", "inv-codex", "sleep"),
            ] {
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \"--home\", \"{}\"]\n",
                    home.display()
                )
                .unwrap();
            }
            write!(
                config,
                "[plan]\nbounces = 0\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{}\"\n[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
                fixture("team-cli.sh").display(),
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
                signals,
                daemon: None,
            }
        }

        /// Starts the daemon and waits for its socket.
        fn start(&mut self) {
            // The socket of a daemon that was killed is still there: a wait for it would end before the new one listens.
            match std::fs::remove_file(self.home.join("agent.sock")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("{error}"),
            }
            let daemon = agent()
                .arg("daemon")
                .arg("--prompts-dir")
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"))
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

        /// Ends the daemon by `signal` and waits for it.
        fn end(&mut self, signal: Signal) {
            let mut daemon = self.daemon.take().unwrap();
            let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
            rustix::process::kill_process(pid, signal).unwrap();
            daemon.wait().unwrap();
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

        /// The causes and heads of the checkpoints of the history, oldest first.
        fn checkpoints(&self) -> Vec<(CheckpointCause, Option<String>)> {
            let Ok(reader) = Reader::open(&self.home.join("history.db")) else {
                return Vec::new();
            };
            let mut taken = Vec::new();
            for team in reader.teams().unwrap() {
                for stored in reader.team_records(&team.value.team).unwrap() {
                    if let Record::Event(event) = stored.record
                        && let HistoryEvent::Checkpoint { cause, head, .. } = event.event
                    {
                        taken.push((cause, head.map(|head| head.to_string())));
                    }
                }
            }
            taken
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            if self.daemon.is_some() {
                self.end(Signal::TERM);
            }
        }
    }

    #[test]
    fn a_daemon_killed_after_a_commit_and_before_the_next_look_records_it_when_it_starts_again() {
        let mut world = World::new();
        world.start();
        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let signals = world.signals.clone();
        eventually("the commit of the run", || signals.join("first").exists());
        let worktree = std::fs::read_dir(world.home.join("worktrees/example"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let head = git(&worktree, &["rev-parse", "HEAD"]);
        assert!(
            !world
                .checkpoints()
                .iter()
                .any(|(_, kept)| kept.as_deref() == Some(head.as_str())),
            "a look saw the commit before the daemon was killed: {:?}",
            world.checkpoints()
        );

        world.end(Signal::KILL);
        world.start();
        eventually("a daemon_started checkpoint of the commit", || {
            world
                .checkpoints()
                .iter()
                .any(|(cause, kept)| *cause == CheckpointCause::DaemonStarted && kept.as_deref() == Some(head.as_str()))
        });
        std::fs::write(signals.join("release"), "").unwrap();
        world.end(Signal::TERM);
    }
}
