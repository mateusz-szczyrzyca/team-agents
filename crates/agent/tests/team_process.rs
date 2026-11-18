//! The `agent` binary as a process, the way a person uses it: `agent daemon`, `agent run` and `agent status` in a
//! temporary `AGENTS_HOME`, on a scratch repository with an `origin`, with a stand-in for the `claude` CLI and the real
//! `agentbus`. The checks of the item "Starting work and the team's life": a solo task and a team started, a worker
//! or too many investigators refused before anything starts, and a member that is done leading to the coordinator's
//! integration, shown by `agent status`.

#[cfg(test)]
mod team_process {
    use std::fmt::Write as _;
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

    /// The stand-in script of the member that posts to its team: it sends one chat message to the team's bus, with the
    /// team read from the prompt of its first run (`.fake-prompt-1`). `AGENTBUS` and `HOME_DIR` are replaced by the test.
    const POST_SCRIPT: &str = "#!/usr/bin/env bash
set -eu
export AGENTS_HOME='HOME_DIR'
team=$(grep -o 'team-[0-9]\\{8\\}-[0-9]\\{6\\}-[a-z0-9]\\{4\\}' .fake-prompt-1 | head -n 1)
'AGENTBUS' send --dir 'HOME_DIR/bus' --from claude-1 --to \"team:$team\" 'Item 1.1 is started by claude-1.'
";

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

    #[test]
    fn debug_e2e_collects_team_with_fake_cli_messages_and_notifications() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "script:post"),
            ("claude-2", "agent", "ask"),
        ]));
        let secrets = world.home.join("secrets.toml");
        std::fs::write(&secrets, "[agent_env]\nAPI_KEY = \"FAKE_KEY\"\n").unwrap();
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).unwrap();
        let scripts = world.home.join("fake-scripts");
        std::fs::create_dir(&scripts).unwrap();
        let post = POST_SCRIPT
            .replace("AGENTBUS", &agentbus_binary().display().to_string())
            .replace("HOME_DIR", &world.home.display().to_string());
        std::fs::write(scripts.join("post.sh"), post).unwrap();
        world.start();

        let output = world.agent(&["run", "inv-opus", "claude-1", "claude-2", "Do it all."]);
        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        let team = text
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();

        world.status_where("the questions to be asked", |status| status.contains("questions"));

        eventually("the notification to be written", || {
            world
                .home
                .join("notifications")
                .read_dir()
                .is_ok_and(|mut entries| entries.next().is_some())
        });

        let diag_parent = world.home.join("diag");
        let debug_output = world.agent(&["debug", "--team", &team, "--out", diag_parent.to_str().unwrap()]);
        assert!(debug_output.status.success(), "{}", stderr(&debug_output));
        let debug_text = stdout(&debug_output);
        assert!(debug_text.contains("Debug summary:"), "{debug_text}");

        let bundle_entry = std::fs::read_dir(&diag_parent).unwrap().next().unwrap().unwrap();
        let bundle_path = bundle_entry.path();

        assert!(bundle_path.join("SUMMARY.md").is_file());
        assert!(bundle_path.join("bus.txt").is_file());
        assert!(bundle_path.join("notifications.txt").is_file());
        assert!(bundle_path.join("agents.toml").is_file());

        let summary = std::fs::read_to_string(bundle_path.join("SUMMARY.md")).unwrap();
        assert!(summary.contains("## Bus Messages"), "{summary}");
        assert!(summary.contains("## Notifications"), "{summary}");
        assert!(summary.contains("questions-1.md"), "{summary}");
        assert!(summary.contains("## Daemon and Versions"), "{summary}");
        assert!(summary.contains("- Configuration:"), "{summary}");
        assert!(summary.contains("Item 1.1 is started by claude-1."), "{summary}");

        let bus = std::fs::read_to_string(bundle_path.join("bus.txt")).unwrap();
        assert!(bus.contains("claude-1 → team"), "{bus}");
        assert!(bus.contains("Item 1.1 is started by claude-1."), "{bus}");

        let notifications = std::fs::read_to_string(bundle_path.join("notifications.txt")).unwrap();
        assert!(notifications.contains("questions-1.md"), "{notifications}");
    }

    fn collect_bundle_files(dir: &Path, files: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir).expect("read_dir on bundle directory must succeed");
        for entry in entries {
            let entry = entry.expect("directory entry must be readable");
            let path = entry.path();
            if path.is_dir() {
                collect_bundle_files(&path, files);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }

    fn populate_untracked_files(worktree: &Path, count: u64) {
        let base_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for i in 1u64..=count {
            let filename = format!("untracked_{i:02}.txt");
            let file_path = worktree.join(&filename);
            std::fs::write(&file_path, format!("content {i}\n")).unwrap();
            let file = std::fs::File::options().write(true).open(&file_path).unwrap();
            let mtime = base_time + std::time::Duration::from_secs(i);
            let times = std::fs::FileTimes::new().set_modified(mtime);
            file.set_times(times).unwrap();
        }
    }

    fn verify_e2e_bundle(bundle_path: &Path) {
        assert!(
            !bundle_path.join("NOT_COLLECTED.txt").exists(),
            "NOT_COLLECTED.txt must not exist"
        );

        let mut bundle_files = Vec::new();
        collect_bundle_files(bundle_path, &mut bundle_files);
        for file in &bundle_files {
            let content = std::fs::read_to_string(file).expect("reading bundle file must succeed");
            assert!(
                !content.contains("Not collected in this version."),
                "file {file:?} must not mention 'Not collected in this version.'"
            );
        }

        let machine_txt = bundle_path.join("machine.txt");
        assert!(machine_txt.is_file(), "machine.txt must exist");
        let machine_content = std::fs::read_to_string(&machine_txt).expect("reading machine.txt must succeed");
        assert!(
            machine_content.contains("## Machine Metrics and Resources"),
            "{machine_content}"
        );
        assert!(
            machine_content.contains("### Daemon Descendant Processes"),
            "{machine_content}"
        );
        assert!(
            machine_content.contains("- Load average: ") && !machine_content.contains("- Load average: unreadable"),
            "machine.txt must report load average values: {machine_content}"
        );
        assert!(
            machine_content.contains("bytes available"),
            "machine.txt must report memory values: {machine_content}"
        );
        assert!(
            machine_content.contains("- Home directory: "),
            "machine.txt must report home directory: {machine_content}"
        );
        assert!(
            machine_content.contains("Space: ") && machine_content.contains("Inodes: "),
            "machine.txt must report filesystem usage: {machine_content}"
        );

        let summary_md = bundle_path.join("SUMMARY.md");
        assert!(summary_md.is_file(), "SUMMARY.md must exist");
        let summary_content = std::fs::read_to_string(&summary_md).expect("reading SUMMARY.md must succeed");
        assert!(
            summary_content.contains("## Machine and Resources"),
            "{summary_content}"
        );
        assert!(
            summary_content.contains("- Load average: ") && !summary_content.contains("- Load average: unknown"),
            "SUMMARY.md must report load average values: {summary_content}"
        );
        assert!(
            summary_content.contains("bytes available"),
            "SUMMARY.md must report memory values: {summary_content}"
        );
        assert!(
            summary_content.contains("- Home filesystem: ") && summary_content.contains("% space"),
            "SUMMARY.md must report home filesystem space percentage: {summary_content}"
        );
        assert!(summary_content.contains("## Git Worktrees"), "{summary_content}");
        assert!(
            summary_content.contains("untracked_55.txt"),
            "newest untracked file must be in summary: {summary_content}"
        );
        assert!(
            !summary_content.contains("untracked_01.txt"),
            "51st newest file must not be in summary: {summary_content}"
        );

        let mut found_untracked_artifact = false;
        let untracked_dir = bundle_path.join("untracked");
        if untracked_dir.is_dir() {
            let entries = std::fs::read_dir(&untracked_dir).expect("read_dir on untracked must succeed");
            for task_entry in entries {
                let task_entry = task_entry.expect("task entry must be readable");
                let untracked_file = task_entry.path();
                if untracked_file.is_file() {
                    let untracked_content =
                        std::fs::read_to_string(&untracked_file).expect("reading untracked.txt must succeed");
                    if untracked_content.contains("untracked_01.txt") && untracked_content.contains("untracked_55.txt")
                    {
                        found_untracked_artifact = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_untracked_artifact,
            "bundle untracked/*.txt must contain all untracked files"
        );
    }

    #[test]
    fn debug_e2e_collects_full_home_machine_and_untracked_with_fake_cli() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "script:post"),
            ("claude-2", "agent", "ask"),
        ]));
        let secrets = world.home.join("secrets.toml");
        std::fs::write(&secrets, "[agent_env]\nAPI_KEY = \"FAKE_KEY\"\n").unwrap();
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).unwrap();
        let scripts = world.home.join("fake-scripts");
        std::fs::create_dir(&scripts).unwrap();
        let post = POST_SCRIPT
            .replace("AGENTBUS", &agentbus_binary().display().to_string())
            .replace("HOME_DIR", &world.home.display().to_string());
        std::fs::write(scripts.join("post.sh"), post).unwrap();
        world.start();

        let output = world.agent(&["run", "inv-opus", "claude-1", "claude-2", "Do it all."]);
        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        let team = text
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();

        world.status_where("the questions to be asked", |status| status.contains("questions"));

        let worktrees_dir = world.home.join("worktrees");
        let mut worktree_path = None;
        let repos = std::fs::read_dir(&worktrees_dir).expect("read_dir on worktrees must succeed");
        for repo_entry in repos {
            let repo_entry = repo_entry.expect("repo entry must be valid");
            let entries = std::fs::read_dir(repo_entry.path()).expect("read_dir on repo path must succeed");
            for entry in entries {
                let entry = entry.expect("task worktree entry must be valid");
                if entry.path().join(".git").exists() {
                    worktree_path = Some(entry.path());
                    break;
                }
            }
            if worktree_path.is_some() {
                break;
            }
        }
        let worktree = worktree_path.expect("at least one task worktree with .git exists");

        populate_untracked_files(&worktree, 55);

        let diag_parent = world.home.join("diag_e2e");
        let debug_output = world.agent(&["debug", "--team", &team, "--out", diag_parent.to_str().unwrap()]);
        assert!(debug_output.status.success(), "{}", stderr(&debug_output));
        let debug_text = stdout(&debug_output);
        assert!(debug_text.contains("Debug summary:"), "{debug_text}");

        let bundle_entry = std::fs::read_dir(&diag_parent)
            .expect("read_dir on diag_e2e must succeed")
            .next()
            .expect("bundle entry must exist")
            .expect("bundle entry must be readable");
        verify_e2e_bundle(&bundle_entry.path());

        std::fs::remove_dir_all(&worktree).unwrap();
        let diag_removed = world.home.join("diag_removed");
        let debug_removed = world.agent(&["debug", "--team", &team, "--out", diag_removed.to_str().unwrap()]);
        assert!(debug_removed.status.success(), "{}", stderr(&debug_removed));
    }
}
