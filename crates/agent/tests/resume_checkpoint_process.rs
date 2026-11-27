//! A machine restart as a process: `agent daemon` and the stand-in for its CLI are killed (`SIGKILL`) in the middle of a
//! run that made a commit and left a file, and the daemon that starts next on the same `AGENTS_HOME` briefs the new
//! session with the `daemon_started` checkpoint: the commit under what is done, the file among the changes, the plan's
//! steps that are not done under what is left, in the same worktree on the same branch at the same head.

#[cfg(test)]
mod resume_checkpoint_process {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of this test lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// The header every briefing of a new session starts with.
    const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";

    /// The stand-in script of the first run: a plan whose step 1.1 is done for R1 and whose step 1.2 is to do for K1, one
    /// commit of step 1.1, an uncommitted file, the process ids of the script and of its parent, and a wait of at most a
    /// minute, so that no run outlives a test that failed.
    const SCRIPT: &str = "set -eu\n\
        sig=\"$SIGNALS\"\n\
        mkdir -p .agent\n\
        printf 'Do it.\\n\\n1. Work\\n[x] 1.1 First\\n    For: R1\\n[ ] 1.2 Second\\n    For: K1\\n' > .agent/plan.md\n\
        git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q --allow-empty -m One -m 'Step: 1.1'\n\
        printf 'draft\\n' > draft.txt\n\
        printf '%s %s\\n' \"$$\" \"$PPID\" > \"$sig/pids\"\n\
        touch \"$sig/midway\"\n\
        for _ in $(seq 1 1200); do [ -e \"$sig/release\" ] && break; sleep 0.05; done\n";

    /// The task list of the run: one item with the requirements `R1` and `K1` that the plan of the script names.
    const LIST: &str = "Build the thing.\n\n1. First\n[ ] 1.1 The first item\n    Do the first thing in the repository.\n    Expected result:\n    - the first thing works\n    Checks:\n    - run the first thing\n";

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

    /// Ends the process `pid` by `signal`.
    fn kill(pid: u32, signal: Signal) {
        let pid = Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        rustix::process::kill_process(pid, signal).unwrap();
    }

    /// The part of `briefing` that is the checkpoint's part `heading`, up to the next heading or the end of the checkpoint.
    fn part<'a>(briefing: &'a str, heading: &str) -> &'a str {
        let (_, rest) = briefing
            .split_once(heading)
            .unwrap_or_else(|| panic!("no {heading}: {briefing}"));
        let to = [rest.find("\n### "), rest.find("\n\nThis is where the work stands.")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(rest.len());
        rest.split_at(to).0
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
        /// A home whose agent runs the script of [`SCRIPT`] in its first run and holds in the next ones.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            let signals = root.join("signals");
            std::fs::create_dir_all(home.join("fake-scripts")).unwrap();
            std::fs::create_dir(&signals).unwrap();
            std::fs::write(
                home.join("fake-scripts/midway.sh"),
                SCRIPT.replace("$SIGNALS", &signals.display().to_string()),
            )
            .unwrap();
            let mut config = String::new();
            for (table, name, scenario) in [
                ("agents", "claude-1", "script:midway,hold"),
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
            kill(daemon.id(), signal);
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

        /// The worktree of the only task of the home.
        fn worktree(&self) -> PathBuf {
            std::fs::read_dir(self.home.join("worktrees/example"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
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
    fn a_machine_restart_in_the_middle_of_a_run_briefs_the_new_session_with_the_commit_and_the_file_it_left() {
        let mut world = World::new();
        world.start();
        std::fs::write(world.repo.join("list.md"), LIST).unwrap();
        let output = world.agent(&["run", "claude-1", "-f", "list.md"]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let signals = world.signals.clone();
        eventually("the commit and the file of the run", || signals.join("midway").exists());
        let worktree = world.worktree();
        let head = git(&worktree, &["rev-parse", "HEAD"]);
        let branch = git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]);

        // The machine goes down: the daemon, and the stand-in with its script, are killed with no word.
        world.end(Signal::KILL);
        let pids = std::fs::read_to_string(signals.join("pids")).unwrap();
        for pid in pids.split_whitespace() {
            kill(pid.parse().unwrap(), Signal::KILL);
        }
        world.start();
        let prompt = worktree.join(".fake-prompt-2");
        eventually("the prompt of the new session", || {
            std::fs::read_to_string(&prompt).is_ok_and(|text| !text.trim().is_empty())
        });
        let briefing = std::fs::read_to_string(&prompt).unwrap();
        let (head_after, branch_after) = (
            git(&worktree, &["rev-parse", "HEAD"]),
            git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        );
        std::fs::write(signals.join("release"), "").unwrap();
        world.end(Signal::TERM);

        assert!(briefing.starts_with(BRIEFING), "{briefing}");
        assert!(briefing.contains("## Checkpoint: daemon_started at "), "{briefing}");
        let short: String = head.chars().take(7).collect();
        let done = part(&briefing, "### What is done\n");
        assert!(
            done.contains(&short),
            "the commit {short} is not under What is done: {done}"
        );
        assert!(done.contains("R1 met"), "{done}");
        let left = part(&briefing, "### What is left\n");
        assert!(left.contains("K1 not started — steps left: 1.2 (not done)"), "{left}");
        assert!(!left.contains("R1"), "something done is listed as left: {left}");
        assert!(
            briefing.contains("draft.txt"),
            "the uncommitted file is not told: {briefing}"
        );
        assert_eq!(
            (head_after, branch_after),
            (head, branch),
            "the same branch at the same head"
        );
    }
}
