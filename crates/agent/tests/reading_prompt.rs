//! Shared reading guidance in the first prompts of real daemon runs with fake CLIs and temporary homes.

#[cfg(test)]
mod reading_prompt {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// The shared guidance heading.
    const SECTION: &str = "## Reading code and command output";

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

    /// A home with a scratch repository `example`, fake agents whose runs keep their prompt in a file of the home, and a
    /// daemon that is started when [`World::start`] is called.
    struct World {
        /// Owns all scratch files until the daemon has stopped.
        _dir: tempfile::TempDir,
        /// Isolated daemon state directory.
        home: PathBuf,
        /// Scratch repository used by CLI commands.
        repo: PathBuf,
        /// Daemon process joined during teardown.
        daemon: Option<Child>,
    }

    impl World {
        /// Creates an isolated repository and fake CLIs that record their first prompt.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            for (table, name) in [
                ("agents", "claude-1"),
                ("agents", "claude-2"),
                ("agents", "glm-1"),
                ("investigators", "inv-opus"),
                ("investigators", "inv-codex"),
                ("investigators", "inv-gemini"),
            ] {
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"claude\"\nargs = [\"--home\", \"{home}\", \"--prompt-file\", \
                     \"{home}/{name}-prompt.txt\", \"--status\", \"STATUS: SLEEPING\"]\n",
                    home = home.display(),
                )
                .unwrap();
            }
            write!(
                config,
                "[plan]\nbounces = 0\n[tiers]\nnormal = [\"claude-1\", \"claude-2\"]\nworkers = [\"glm-1\"]\n\
                 [binaries]\nclaude = \"{}\"\n\
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
    fn a_solo_first_prompt_places_reading_guidance_inside_the_working_context() {
        let mut world = World::new();
        world.start();
        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8(output.stderr).unwrap());

        let prompt = world.first_prompt("claude-1");
        let context = prompt.find("# Working context").expect("working context");
        let section = prompt.find(SECTION).expect("reading guidance");
        let ending = prompt.find("## Ending your turn").expect("ending instructions");
        assert!(
            context < section && section < ending,
            "guidance belongs inside the context"
        );
    }

    #[test]
    fn a_reviewer_prompt_contains_reading_guidance() {
        let mut world = World::new();
        world.start();
        let output = world.agent(&["run", "inv-opus", "Review the pull request."]);
        assert!(output.status.success(), "{}", String::from_utf8(output.stderr).unwrap());

        let prompt = world.first_prompt("inv-opus");
        assert!(prompt.contains("You are agent **inv-opus**"));
        assert!(prompt.contains(SECTION), "reviewer receives reading guidance");
    }

    #[test]
    fn a_worker_prompt_contains_reading_guidance() {
        let mut world = World::new();
        world.start();
        let list = world.repo.join("list.md");
        std::fs::write(
            &list,
            "Fix the README.\n\n1. Fix\n[ ] 1.1 Fix the typo\n    Fix the README.\n",
        )
        .unwrap();
        let output = world.agent(&[
            "run",
            "claude-1",
            "claude-2",
            "glm-1",
            "--coordinator",
            "claude-1",
            "-f",
            list.to_str().unwrap(),
        ]);
        assert!(output.status.success(), "{}", String::from_utf8(output.stderr).unwrap());

        let prompt = world.first_prompt("glm-1");
        assert!(prompt.contains("Your role: worker"), "the run is a worker");
        assert!(prompt.contains(SECTION), "worker receives reading guidance");
    }

    #[test]
    fn reading_guidance_limits_git_diffs_and_command_dumps() {
        let mut world = World::new();
        world.start();
        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8(output.stderr).unwrap());

        let prompt = world.first_prompt("claude-1");
        let section = prompt.split(SECTION).nth(1).expect("reading guidance");
        let section = section.split("## Ending your turn").next().expect("section body");
        assert!(section.contains("git diff --stat"));
        assert!(section.contains("One dump per command"));
    }
}
