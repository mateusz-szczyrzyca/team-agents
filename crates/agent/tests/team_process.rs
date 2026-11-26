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
                "[plan]\nbounces = 0\n[tiers]\nnormal = [{}]\nworkers = [{}]\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n",
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

        /// Stops the daemon and starts it again on the same home, as after a restart of the machine's service.
        fn restart(&mut self) {
            if let Some(mut daemon) = self.daemon.take() {
                let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
                rustix::process::kill_process(pid, Signal::TERM).unwrap();
                daemon.wait().unwrap();
            }
            self.start();
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
        let (version, core, runtime, web) = (
            env!("CARGO_PKG_VERSION"),
            agent_core::VERSION,
            agent_runtime::VERSION,
            agent_web::VERSION,
        );
        assert_eq!(
            world.status(),
            format!("agent {version} · agent-core {core} · agent-runtime {runtime} · agent-web {web}\nno teams\n")
        );
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

    /// The task of the solo run that `output` started: the first word of its second line.
    fn solo_task(output: &Output) -> String {
        assert!(output.status.success(), "{}", stderr(output));
        stdout(output)
            .lines()
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned()
    }

    /// The line of `status` about `task`.
    fn line_of<'a>(status: &'a str, task: &str) -> &'a str {
        status.lines().find(|line| line.contains(task)).unwrap_or_default()
    }

    /// Appends `text` to the configuration of the home, before the daemon starts.
    fn configure(world: &World, text: &str) {
        let path = world.home.join("agents.toml");
        let mut config = std::fs::read_to_string(&path).unwrap();
        config.push_str(text);
        std::fs::write(path, config).unwrap();
    }

    /// Lets the run of `task` end: the `hold` scenario waits for this file in the worktree of the task.
    fn release(world: &World, task: &str) {
        std::fs::write(
            world.home.join("worktrees/example").join(task).join(".fake-continue"),
            "",
        )
        .unwrap();
    }

    #[test]
    fn a_task_whose_agent_runs_in_another_task_waits_for_that_run_and_starts_after_it() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        world.start();
        let first = solo_task(&world.agent(&["run", "claude-1", "First."]));
        world.status_where("the first run to go", |status| {
            line_of(status, &first).contains("running")
        });

        let second = solo_task(&world.agent(&["run", "claude-1", "Second."]));

        let status = world.status_where("the second task to wait for its agent", |status| {
            line_of(status, &second).contains("queued, waits for claude-1, running in team-")
        });
        assert!(line_of(&status, &first).contains("running"), "{status}");
        assert_eq!(world.runs(&second), 0, "{status}");
        assert!(!line_of(&status, &second).contains("failed"), "{status}");

        release(&world, &first);
        eventually("the second task to start after the first run", || {
            world.runs(&second) == 1
        });
        let status = world.status_where("the second task to run", |status| {
            line_of(status, &second).contains("running")
        });
        assert!(!line_of(&status, &second).contains("waits for"), "{status}");
        assert_eq!(world.runs(&first), 1);
        release(&world, &second);
    }

    #[test]
    fn a_limit_of_no_runs_starts_both_tasks_of_one_agent_at_once() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        configure(&world, "\n[limits]\nruns_per_agent = 0\n");
        world.start();
        let first = solo_task(&world.agent(&["run", "claude-1", "First."]));
        let second = solo_task(&world.agent(&["run", "claude-1", "Second."]));

        let status = world.status_where("both tasks to run", |status| {
            line_of(status, &first).contains("running") && line_of(status, &second).contains("running")
        });
        assert!(!status.contains("waits for"), "{status}");
        eventually("both runs to go", || {
            world.runs(&first) == 1 && world.runs(&second) == 1
        });
        release(&world, &first);
        release(&world, &second);
    }

    #[test]
    fn a_restart_of_the_daemon_keeps_the_wait_of_a_task_for_its_agent() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        world.start();
        let first = solo_task(&world.agent(&["run", "claude-1", "First."]));
        world.status_where("the first run to go", |status| {
            line_of(status, &first).contains("running")
        });
        let second = solo_task(&world.agent(&["run", "claude-1", "Second."]));
        world.status_where("the second task to wait", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });

        world.restart();

        let status = world.status_where("the second task to wait after the restart", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");
        release(&world, &first);
        eventually("the second task to start after the restart", || {
            world.runs(&second) == 1
        });
        release(&world, &second);
    }

    #[test]
    fn an_old_state_with_two_runs_of_one_agent_going_loads_and_both_finish() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        configure(&world, "\n[limits]\nruns_per_agent = 0\n");
        world.start();
        let first = solo_task(&world.agent(&["run", "claude-1", "First."]));
        let second = solo_task(&world.agent(&["run", "claude-1", "Second."]));
        world.status_where("both tasks to run in the state before the limit", |status| {
            line_of(status, &first).contains("running") && line_of(status, &second).contains("running")
        });

        // Running status precedes the fake CLI's counter write; both initial invocations must be recorded.
        let initial_prompts =
            [&first, &second].map(|task| world.home.join("worktrees/example").join(task).join(".fake-prompt-1"));
        let deadline = Instant::now() + PATIENCE;
        while !initial_prompts.iter().all(|path| path.exists()) {
            assert!(
                Instant::now() < deadline,
                "both fake CLIs must record their initial invocation"
            );
            std::thread::yield_now();
        }

        let config_path = world.home.join("agents.toml");
        let without_zero = std::fs::read_to_string(&config_path)
            .unwrap()
            .replace("\n[limits]\nruns_per_agent = 0\n", "");
        std::fs::write(&config_path, without_zero).unwrap();
        world.restart();

        let status = world.status_where(
            "the first task to run and the second to wait under the new default limit",
            |status| {
                line_of(status, &first).contains("running")
                    && line_of(status, &second).contains("queued, waits for claude-1, running in team-")
            },
        );
        assert!(!status.contains("failed"), "{status}");

        release(&world, &first);
        world.status_where("the first task to finish its run and the second to run", |status| {
            line_of(status, &first).contains("waiting") && line_of(status, &second).contains("running")
        });
        release(&world, &second);
        world.status_where("both runs of the old state to finish", |status| {
            line_of(status, &first).contains("waiting") && line_of(status, &second).contains("waiting")
        });
        assert_eq!(world.runs(&first), 2);
        assert_eq!(world.runs(&second), 2);
    }

    #[test]
    fn a_resumed_task_of_an_older_team_starts_after_an_already_waiting_queued_task() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        world.start();
        let a = solo_task(&world.agent(&["run", "claude-1", "Task A."]));
        world.status_where("task A to run", |status| line_of(status, &a).contains("running"));
        release(&world, &a);
        world.status_where("task A to finish its first run", |status| {
            line_of(status, &a).contains("waiting")
        });
        std::fs::remove_file(world.home.join("worktrees/example").join(&a).join(".fake-continue")).unwrap();

        let b = solo_task(&world.agent(&["run", "claude-1", "Task B."]));
        world.status_where("task B to run", |status| line_of(status, &b).contains("running"));

        let c = solo_task(&world.agent(&["run", "claude-1", "Task C."]));
        world.status_where("task C to wait for task B", |status| {
            line_of(status, &c).contains("queued, waits for claude-1, running in team-")
        });

        let resume = world.agent(&["resume", &a]);
        assert!(resume.status.success(), "{}", stderr(&resume));
        world.status_where("task A to be queued behind task B", |status| {
            line_of(status, &a).contains("queued, waits for claude-1, running in team-")
        });

        world.restart();
        eventually("task B to resume after the restart", || world.runs(&b) == 2);

        release(&world, &b);
        eventually("task C to start before resumed task A", || world.runs(&c) == 1);
        let status = world.status_where("task C to start before resumed task A", |status| {
            line_of(status, &c).contains("running") && line_of(status, &a).contains("queued, waits for claude-1")
        });
        assert_eq!(world.runs(&c), 1, "{status}");
        assert_eq!(world.runs(&a), 1, "{status}");

        release(&world, &c);
        eventually("task A to start after task C finishes", || world.runs(&a) == 2);
        world.status_where("task A to start after task C finishes", |status| {
            line_of(status, &a).contains("running")
        });
        release(&world, &a);
    }

    #[test]
    fn a_local_round_without_github_names_the_busy_agent_and_team_in_status() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        world.start();
        let first_run = world.agent(&["run", "claude-1", "First local solo."]);
        let first_team = stdout(&first_run)
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();
        let first = solo_task(&first_run);
        world.status_where("the first solo task to run", |status| {
            line_of(status, &first).contains("running")
        });

        let second = solo_task(&world.agent(&["run", "claude-1", "Second local solo."]));
        let expected_round_wait = format!(
            "waits for the queued run of its task to start, owed by the harness, after the run of claude-1 in {first_team}"
        );
        let status = world.status_where("the second round to name the busy agent and team", |status| {
            status.contains(&expected_round_wait)
        });
        assert!(
            !status.contains("waits for an answer of the human"),
            "a local queued round must not say it waits for the human: {status}"
        );

        release(&world, &first);
        eventually("the second solo task's run to start", || world.runs(&second) == 1);
        world.status_where("the second solo task to run", |status| {
            line_of(status, &second).contains("running")
        });
        release(&world, &second);
    }

    /// The prompt of the first run of `task`, which the stand-in CLI keeps in the worktree of the task.
    fn first_prompt(world: &World, task: &str) -> String {
        std::fs::read_to_string(world.home.join("worktrees/example").join(task).join(".fake-prompt-1"))
            .unwrap_or_default()
    }

    /// Starts `first` (held by `agent` until released) and `second` for the same agent, which waits for it.
    fn busy_agent_with_a_second_task(world: &World, agent: &str) -> (String, String) {
        let first = solo_task(&world.agent(&["run", agent, "First."]));
        world.status_where("the first run to go", |status| {
            line_of(status, &first).contains("running")
        });
        let second = solo_task(&world.agent(&["run", agent, "Second."]));
        world.status_where("the second task to wait", |status| {
            line_of(status, &second).contains(&format!("waits for {agent}"))
        });
        (first, second)
    }

    /// As [`busy_agent_with_a_second_task`], for a configuration that hands the second task over after a second of
    /// waiting: under load the status may not show the wait before the hand-over, so the wait is the row the daemon
    /// logs once when it finds the task waiting, which stays.
    fn busy_agent_with_a_second_task_that_may_be_handed_over(world: &World, agent: &str) -> (String, String) {
        let first = solo_task(&world.agent(&["run", agent, "First."]));
        world.status_where("the first run to go", |status| {
            line_of(status, &first).contains("running")
        });
        let second = solo_task(&world.agent(&["run", agent, "Second."]));
        eventually("the daemon to log that the second task waits for its agent", || {
            log_rows(world, "task_waits_for_agent", &second) > 0
        });
        (first, second)
    }

    /// Stops the daemon, sets the recorded `waiting_since` in `state.json` to an earlier moment so the wait exceeds
    /// `busy_after` without a wall-clock sleep, and starts the daemon again. With a distant `busy_after` (tests 6 and
    /// 12's configuration) the daemon cannot hand the task over before this deliberate expiry.
    fn expire_recorded_wait(world: &mut World) {
        // The timestamp is written when the daemon first decides the task waits, before it ever serves a status that
        // says so: wait for the persist on disk while the daemon still runs, so the expiry below always happens after
        // it, never racing it.
        eventually("waiting_since to be persisted in state.json", || {
            let file = world.home.join("state.json");
            match std::fs::read_to_string(&file) {
                Ok(text) => {
                    agent_core::state::State::from_json(&text).unwrap();
                    text.contains("\"waiting_since\": \"")
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => panic!("cannot read saved state {}: {error}", file.display()),
            }
        });
        if let Some(mut daemon) = world.daemon.take() {
            let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
            rustix::process::kill_process(pid, Signal::TERM).unwrap();
            daemon.wait().unwrap();
        }
        let state_path = world.home.join("state.json");
        let state_text = std::fs::read_to_string(&state_path).unwrap();
        assert!(
            state_text.contains("\"waiting_since\": \""),
            "waiting_since must be persisted in state.json:\n{state_text}"
        );
        let updated: String = state_text
            .lines()
            .map(|line| {
                if let Some((prefix, _)) = line.split_once("\"waiting_since\":") {
                    format!("{prefix}\"waiting_since\": \"2020-01-01T00:00:00Z\"")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&state_path, updated).unwrap();
        world.start();
    }

    #[test]
    fn a_task_that_waits_too_long_for_its_busy_agent_is_handed_over_to_a_free_agent() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "done"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
        world.start();
        let (first, second) = busy_agent_with_a_second_task_that_may_be_handed_over(&world, "claude-1");

        let status = world.status_where("the second task to be handed over and done", |status| {
            line_of(status, &second).contains("claude-2") && line_of(status, &second).contains("done")
        });
        assert!(line_of(&status, &second).contains("for claude-1"), "{status}");
        assert_eq!(world.runs(&second), 1, "{status}");
        let prompt = first_prompt(&world, &second);
        assert!(prompt.contains("busy"), "the briefing says why: {prompt}");
        assert!(
            world
                .home
                .join("worktrees/example")
                .join(&second)
                .join(".fake-runs")
                .is_file(),
            "the new holder ran in the worktree of the task"
        );
        assert_eq!(world.runs(&first), 1);

        // The first holder is not woken for the task when it becomes free.
        release(&world, &first);
        world.status_where("the first task to finish its run", |status| {
            line_of(status, &first).contains("waiting")
        });
        assert_eq!(world.runs(&second), 1, "{}", world.status());
    }

    #[test]
    fn a_task_waits_for_its_busy_agent_when_no_agent_of_the_replacement_list_fits() {
        let mut world = World::new(&agents_with(&[("claude-1", "agent", "hold")]));
        configure(&world, "\n[replacement]\nagents = []\nbusy_after = 1\n");
        world.start();
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        expire_recorded_wait(&mut world);
        let status = world.status_where("the second task to still wait after busy_after", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");

        release(&world, &first);
        eventually("the second task to start with its own agent", || {
            world.runs(&second) == 1
        });
        let status = world.status_where("the second task to run", |status| {
            line_of(status, &second).contains("running")
        });
        assert!(!line_of(&status, &second).contains("for "), "{status}");
        release(&world, &second);
    }

    #[test]
    fn a_zero_busy_after_never_hands_a_task_over() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "done"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 0\n");
        world.start();
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        expire_recorded_wait(&mut world);
        let status = world.status_where("the second task to still wait when busy_after is 0", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");
        release(&world, &first);
        eventually("the second task to start with its own agent", || {
            world.runs(&second) == 1
        });
        release(&world, &second);
    }

    /// How many rows of the daemon's log of `event` concern `task`, read with `agent log`.
    fn log_rows(world: &World, event: &str, task: &str) -> usize {
        let output = world.agent(&["log", "--event", event, "--task", task, "--json"]);
        assert!(output.status.success(), "{}", stderr(&output));
        stdout(&output).lines().filter(|line| !line.trim().is_empty()).count()
    }

    /// Expires the recorded wait of `task` (see [`expire_recorded_wait`]) and waits until the daemon has looked at the
    /// wait again. A restarted daemon logs `task_waits_for_agent` for each task it finds waiting, so one more such row
    /// is the observable end of a scheduling cycle after `busy_after` has passed. Asserts that nothing was handed over.
    fn expire_and_wait_for_the_daemon_to_look(world: &mut World, task: &str) {
        let before = log_rows(world, "task_waits_for_agent", task);
        expire_recorded_wait(world);
        let world = &*world;
        eventually("the daemon to look at the wait again after busy_after", || {
            log_rows(world, "task_waits_for_agent", task) > before
        });
        assert_eq!(log_rows(world, "task_handed_over", task), 0, "{}", world.status());
    }

    #[test]
    fn a_worker_of_a_lower_tier_is_not_given_a_waiting_task() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("worker-1", "worker", "done"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"worker-1\"]\nbusy_after = 1\n");
        world.start();
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        expire_and_wait_for_the_daemon_to_look(&mut world, &second);
        let status = world.status_where("the second task to still wait after busy_after", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");
        assert!(!line_of(&status, &second).contains("worker-1"), "{status}");

        release(&world, &first);
        eventually("the second task to start with its own agent", || {
            world.runs(&second) == 1
        });
        let status = world.status_where("the second task to run", |status| {
            line_of(status, &second).contains("running")
        });
        assert!(!line_of(&status, &second).contains("for "), "{status}");
        release(&world, &second);
    }

    #[test]
    fn a_running_candidate_is_not_given_a_waiting_task() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "hold"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
        world.start();
        let busy_candidate = solo_task(&world.agent(&["run", "claude-2", "Its own work."]));
        world.status_where("the candidate's own run to go", |status| {
            line_of(status, &busy_candidate).contains("running")
        });
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        expire_and_wait_for_the_daemon_to_look(&mut world, &second);
        let status = world.status_where("the second task to still wait after busy_after", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");
        assert!(
            line_of(&status, &busy_candidate).contains("running")
                && !line_of(&status, &busy_candidate).contains("for "),
            "the candidate keeps its own run, which the restart of the daemon resumes: {status}"
        );

        release(&world, &first);
        eventually("the second task to start with its own agent", || {
            world.runs(&second) == 1
        });
        release(&world, &second);
        release(&world, &busy_candidate);
    }

    #[test]
    fn a_candidate_of_the_tasks_own_team_is_not_given_a_waiting_task() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "sleep"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
        world.start();
        let first = solo_task(&world.agent(&["run", "claude-1", "First."]));
        world.status_where("the first run to go", |status| {
            line_of(status, &first).contains("running")
        });
        let list = world.repo.join("list.md");
        std::fs::write(&list, LIST).unwrap();
        let team = world.agent(&["run", "claude-1", "inv-opus", "claude-2", "-f", list.to_str().unwrap()]);
        assert!(team.status.success(), "{}", stderr(&team));
        let status = world.status_where("the team's member on claude-1 to wait for it", |status| {
            status
                .lines()
                .any(|line| line.contains("waits for claude-1") && !line.contains(&first))
        });
        let second = status
            .lines()
            .find(|line| line.contains("waits for claude-1") && !line.contains(&first))
            .and_then(|line| line.split_whitespace().find(|word| word.starts_with("task-")))
            .unwrap()
            .to_owned();

        expire_and_wait_for_the_daemon_to_look(&mut world, &second);
        let status = world.status_where("the member to still wait after busy_after", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert!(!line_of(&status, &second).contains("claude-2"), "{status}");
        assert_eq!(world.runs(&second), 0, "{status}");

        release(&world, &first);
        eventually("the member to start with its own agent", || world.runs(&second) == 1);
    }

    #[test]
    fn a_coordinator_whose_member_runs_is_not_handed_over_for_its_busy_wait() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "hold"),
            ("claude-3", "agent", "done"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-3\"]\nbusy_after = 1\n");
        world.start();
        let first = solo_task(&world.agent(&["run", "claude-1", "First."]));
        world.status_where("the first run to go", |status| {
            line_of(status, &first).contains("running")
        });
        let list = world.repo.join("list.md");
        std::fs::write(&list, LIST).unwrap();
        let team = world.agent(&[
            "run",
            "claude-2",
            "claude-1",
            "--coordinator",
            "claude-1",
            "-f",
            list.to_str().unwrap(),
        ]);
        assert!(team.status.success(), "{}", stderr(&team));
        let status = world.status_where("the coordinator to wait for claude-1 while its member runs", |status| {
            status
                .lines()
                .any(|line| line.contains("coordinator") && line.contains("waits for claude-1"))
                && status
                    .lines()
                    .any(|line| line.contains("member") && line.contains("claude-2") && line.contains("running"))
        });
        let lead = status
            .lines()
            .find(|line| line.contains("coordinator") && line.contains("waits for claude-1"))
            .and_then(|line| line.split_whitespace().find(|word| word.starts_with("task-")))
            .unwrap()
            .to_owned();

        expire_and_wait_for_the_daemon_to_look(&mut world, &lead);
        let status = world.status_where("the coordinator to still wait", |status| {
            line_of(status, &lead).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&lead), 0, "{status}");
        assert!(!line_of(&status, &lead).contains("claude-3"), "{status}");

        release(&world, &first);
        eventually("the coordinator to start with its own agent", || world.runs(&lead) == 1);
    }

    /// Lets `waiting` (queued for `claude-1`) be handed over after `busy_after` has passed, and waits for that
    /// hand-over: the positive control that the scheduler looked at the waits of the test after they expired.
    fn expire_and_wait_for_the_hand_over(world: &mut World, waiting: &str) {
        expire_recorded_wait(world);
        let world = &*world;
        eventually("the positive control to be handed over after busy_after", || {
            log_rows(world, "task_handed_over", waiting) > 0
        });
    }

    #[test]
    fn a_running_task_is_not_handed_over_while_a_free_candidate_exists() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "done"),
        ]));
        // Only `expire_recorded_wait` may expire the waits here: a short busy_after would let the daemon hand the
        // waiting task over before the test kills it, racing the deliberate expiry below.
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 3600\n");
        world.start();
        let running = solo_task(&world.agent(&["run", "claude-1", "Its own work."]));
        world.status_where("the running task to go", |status| {
            line_of(status, &running).contains("running")
        });
        // The positive control: a second task of claude-1 waits for the running one and goes to the free candidate.
        let waiting = solo_task(&world.agent(&["run", "claude-1", "Waits."]));
        world.status_where("the second task to wait", |status| {
            line_of(status, &waiting).contains("waits for claude-1")
        });

        expire_and_wait_for_the_hand_over(&mut world, &waiting);
        let status = world.status_where("the running task to keep its agent", |status| {
            line_of(status, &running).contains("running")
        });
        assert_eq!(log_rows(&world, "task_handed_over", &running), 0, "{status}");
        assert!(!line_of(&status, &running).contains("claude-2"), "{status}");

        release(&world, &running);
    }

    #[test]
    fn a_task_that_waits_for_an_answer_of_the_human_is_not_handed_over() {
        let mut world = World::new(&agents_with(&[
            ("claude-2", "agent", "done"),
            ("claude-3", "agent", "ask-or-hold"),
        ]));
        // Only `expire_recorded_wait` may expire the waits here: a short busy_after would let the daemon hand the
        // waiting task over before the test kills it, racing the deliberate expiry below.
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 3600\n");
        world.start();
        let asking = solo_task(&world.agent(&["run", "claude-3", "Asks the human."]));
        world.status_where("the task to ask the human", |status| {
            line_of(status, &asking).contains("questions")
        });
        // A run of claude-3 holds it, so the next task waits for it while the asking one waits for the human.
        let holder = solo_task(&world.agent(&["run", "claude-3", "Holds the agent."]));
        world.status_where("the holder to run", |status| {
            line_of(status, &holder).contains("running")
        });
        let waiting = solo_task(&world.agent(&["run", "claude-3", "Waits."]));
        world.status_where("the third task to wait", |status| {
            line_of(status, &waiting).contains("waits for claude-3")
        });

        expire_and_wait_for_the_hand_over(&mut world, &waiting);
        let status = world.status_where("the asking task to keep waiting for the human", |status| {
            line_of(status, &asking).contains("questions")
        });
        assert_eq!(log_rows(&world, "task_handed_over", &asking), 0, "{status}");
        assert!(!line_of(&status, &asking).contains("claude-2"), "{status}");

        release(&world, &holder);
    }

    #[test]
    fn a_paused_candidate_is_not_given_a_waiting_task() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "turn-limit-quota"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
        world.start();
        let quota_hit = solo_task(&world.agent(&["run", "claude-2", "Its own work."]));
        world.status_where("the candidate to pause for its quota", |status| {
            line_of(status, &quota_hit).contains("paused")
        });
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        expire_and_wait_for_the_daemon_to_look(&mut world, &second);
        let status = world.status_where("the second task to still wait after busy_after", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");
        assert!(line_of(&status, &quota_hit).contains("paused"), "{status}");

        release(&world, &first);
        eventually("the second task to start with its own agent", || {
            world.runs(&second) == 1
        });
        release(&world, &second);
    }

    #[test]
    fn a_refused_agent_gets_no_waiting_task_while_its_pause_lasts() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "turn-limit-quota"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
        world.start();
        let quota_hit = solo_task(&world.agent(&["run", "claude-2", "Its own work."]));
        world.status_where("the candidate to be refused for its quota", |status| {
            line_of(status, &quota_hit).contains("paused")
        });
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        expire_and_wait_for_the_daemon_to_look(&mut world, &second);
        let status = world.status_where("the second task to keep waiting for claude-1", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0, "{status}");
        assert!(line_of(&status, &quota_hit).contains("paused"), "{status}");

        release(&world, &first);
        release(&world, &second);
    }

    #[test]
    fn a_round_whose_task_is_paused_by_its_provider_says_which_provider_and_not_the_human() {
        let mut world = World::new(&agents_with(&[("claude-2", "agent", "turn-limit-quota")]));
        world.start();
        let refused = solo_task(&world.agent(&["run", "claude-2", "Its own work."]));

        let status = world.status_where("the round of the refused task to wait at its provider", |status| {
            status.contains("waits for the provider of claude-2 until ")
        });

        assert!(line_of(&status, &refused).contains("paused"), "{status}");
        assert!(
            !status.contains("waits for an answer of the human"),
            "a refusal by the provider is not the human's to answer: {status}"
        );
    }

    #[test]
    fn a_refused_agent_gets_a_waiting_task_after_its_return_and_a_good_retry() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "turn-limit-quota,done"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n");
        world.start();
        let quota_hit = solo_task(&world.agent(&["run", "claude-2", "Its own work."]));
        world.status_where("the candidate to be refused for its quota", |status| {
            line_of(status, &quota_hit).contains("paused")
        });
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");
        expire_and_wait_for_the_daemon_to_look(&mut world, &second);
        world.status_where("the second task to keep waiting for claude-1", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        assert_eq!(world.runs(&second), 0);

        move_the_refusal_and_the_pause_back(&mut world, "claude-2");
        let status = world.status_where("the paused task to be done after its good retry", |status| {
            line_of(status, &quota_hit).contains("done")
        });
        assert_eq!(world.runs(&quota_hit), 2, "{status}");
        let status = world.status_where("the second task to run on claude-2 after its return", |status| {
            line_of(status, &second).contains("claude-2")
        });
        assert_eq!(world.runs(&second), 1, "{status}");

        release(&world, &first);
    }

    #[test]
    fn a_fresh_task_of_a_refused_agent_is_not_started_before_its_return() {
        let mut world = World::new(&agents_with(&[("claude-2", "agent", "turn-limit-quota,done")]));
        world.start();
        let quota_hit = solo_task(&world.agent(&["run", "claude-2", "Its own work."]));
        world.status_where("the candidate to be refused for its quota", |status| {
            line_of(status, &quota_hit).contains("paused")
        });
        let fresh = solo_task(&world.agent(&["run", "claude-2", "Fresh work."]));

        never_started_within(&world, &fresh, Duration::from_secs(3));
        let status = world.status();
        assert!(line_of(&status, &quota_hit).contains("paused"), "{status}");

        move_the_refusal_and_the_pause_back(&mut world, "claude-2");
        eventually("the fresh task to run on claude-2 after its return", || {
            world.runs(&fresh) == 1
        });
    }

    #[test]
    fn a_deepinfra_refusal_pauses_the_task_and_its_notice_tells_the_provider_words_and_the_local_resume_time() {
        // The daemon runs in UTC+01:00 (a POSIX zone string, so no zone database is needed): the notice must give its
        // resume time in that zone, with no offset after it, never in UTC.
        let mut world = deepinfra_world("deepinfra-overload");
        start_in_zone(&mut world, "<+01>-1");

        let task = solo_task(&world.agent(&["run", "deep-1", "Its own work."]));
        let status = world.status_where("the task to be paused for the refusal", |status| {
            line_of(status, &task).contains("paused")
        });
        assert!(line_of(&status, &task).contains("paused"), "{status}");

        eventually("the notice of the refusal to be written", || {
            std::fs::read_dir(world.home.join("notifications")).is_ok_and(|entries| {
                entries.into_iter().any(|entry| {
                    std::fs::read_to_string(entry.unwrap().path())
                        .unwrap()
                        .contains("refused its run")
                })
            })
        });
        let notice: String = std::fs::read_dir(world.home.join("notifications"))
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert!(notice.contains("refused its run: an overload (rate_limit)"), "{notice}");
        assert!(
            notice.contains("429 · Model busy, retry later · engine_overloaded"),
            "{notice}"
        );
        assert!(notice.contains("paused until "), "{notice}");
        assert!(!notice.contains("+01:00"), "{notice}");
        assert!(!notice.contains("UTC"), "{notice}");
        assert!(!notice.contains('Z'), "{notice}");
    }

    /// A home of one `opencode` agent `deep-1` on a `DeepInfra` model, running `scenario` through the stand-in. It is a
    /// home of its own: `World::new` writes only `claude` agents, so this test replaces the configuration it wrote.
    fn deepinfra_world(scenario: &str) -> World {
        let world = World::new(&agents_with(&[]));
        let home = world.home.display().to_string();
        let bus = agentbus_binary().display().to_string();
        let cli = script("team-cli.sh").display().to_string();
        let mut config = format!(
            "[agents.deep-1]\ncli = \"opencode\"\nmodel = \"deepinfra/zai-org/GLM-5.3-Flash\"\n\
             args = [\"--scenario\", \"{scenario}\", \"--as\", \"deep-1\", \"--bus-bin\", \"{bus}\", \
             \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n\
             [plan]\nbounces = 0\n[tiers]\nnormal = [\"deep-1\"]\nworkers = []\n\
             [binaries]\nclaude = \"{cli}\"\nopencode = \"{cli}\"\nagentbus = \"{bus}\"\n\
             [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n"
        );
        // The daemon needs two investigators outside the team for every pull request: the reviewers of the other tests.
        for (name, _, scenario) in REVIEWERS {
            write!(
                config,
                "[investigators.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \
                 \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n"
            )
            .unwrap();
        }
        std::fs::write(world.home.join("agents.toml"), config).unwrap();
        world
    }

    /// Starts the daemon of `world` with the time zone `tz`, as [`World::start`] starts it otherwise.
    fn start_in_zone(world: &mut World, tz: &str) {
        let daemon = agent()
            .arg("daemon")
            .arg("--prompts-dir")
            .arg(prompts())
            .env("AGENTS_HOME", &world.home)
            .env("TZ", tz)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let socket = world.home.join("agent.sock");
        eventually("the socket", || socket.exists());
        world.daemon = Some(daemon);
    }

    /// Checks for `window` that `task` has no run: the daemon is given the time to start it if it would.
    fn never_started_within(world: &World, task: &str, window: Duration) {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            assert_eq!(world.runs(task), 0, "{}", world.status());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Moves every ended run and the return time of `agent` back to 2020, as if the time had passed: the daemon is
    /// stopped, the file is edited, and the daemon starts again. The paused task is woken at once and its next run is
    /// the good one.
    fn move_the_refusal_and_the_pause_back(world: &mut World, agent: &str) {
        if let Some(mut daemon) = world.daemon.take() {
            let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
            rustix::process::kill_process(pid, Signal::TERM).unwrap();
            daemon.wait().unwrap();
        }
        let state_path = world.home.join("state.json");
        let state_text = std::fs::read_to_string(&state_path).unwrap();
        let key = format!("\"{agent}\": \"");
        let mut lines: Vec<String> = state_text.lines().map(str::to_owned).collect();
        let mut in_refusals = false;
        let mut last_started: Option<usize> = None;
        let mut ended = 0;
        let mut refusals = 0;
        for index in 0..lines.len() {
            let line = lines[index].clone();
            let trimmed = line.trim_start();
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            let comma = if line.trim_end().ends_with(',') { "," } else { "" };
            if line.contains("\"refused_agents\": {") {
                in_refusals = true;
            } else if in_refusals && trimmed.starts_with('}') {
                in_refusals = false;
            } else if in_refusals && trimmed.starts_with(&key) {
                refusals += 1;
                lines[index] = format!("{indent}\"{agent}\": \"2020-01-01T00:00:00Z\"{comma}");
            } else if trimmed.starts_with("\"started_at\": \"") {
                last_started = Some(index);
            } else if trimmed == "\"status\": \"ended\"," {
                ended += 1;
                if let Some(started) = last_started {
                    let started_indent: String = lines[started].chars().take_while(|c| c.is_whitespace()).collect();
                    let started_comma = if lines[started].trim_end().ends_with(',') {
                        ","
                    } else {
                        ""
                    };
                    lines[started] = format!("{started_indent}\"started_at\": \"2019-12-31T00:00:00Z\"{started_comma}");
                }
            } else if trimmed.starts_with("\"at\": \"") && lines[index - 1].trim_start() == "\"status\": \"ended\"," {
                lines[index] = format!("{indent}\"at\": \"2020-01-01T00:00:00Z\"{comma}");
            }
        }
        assert!(ended >= 1 && refusals == 1, "unexpected state:\n{state_text}");
        std::fs::write(&state_path, lines.join("\n")).unwrap();
        world.start();
    }

    #[test]
    fn a_restart_during_the_wait_keeps_the_time_the_task_has_waited() {
        let mut world = World::new(&agents_with(&[
            ("claude-1", "agent", "hold"),
            ("claude-2", "agent", "done"),
        ]));
        configure(&world, "\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 300\n");
        world.start();
        let (first, second) = busy_agent_with_a_second_task(&world, "claude-1");

        world.restart();
        world.status_where("the second task to still wait after the restart", |status| {
            line_of(status, &second).contains("waits for claude-1")
        });
        expire_recorded_wait(&mut world);

        world.status_where(
            "the second task to be handed over after the recorded wait expires",
            |status| line_of(status, &second).contains("for claude-1") && line_of(status, &second).contains("done"),
        );
        assert_eq!(world.runs(&second), 1);
        release(&world, &first);
    }
    /// A member refused by `DeepInfra` and a replacement that requires takeover context.
    fn refused_member_world() -> World {
        let world = World::new(&agents_with(&[
            ("deep-1", "agent", "deepinfra-overload"),
            ("claude-2", "agent", "sleep"),
            (
                "claude-3",
                "agent",
                "work-if-briefed:A NEW SESSION ON A TASK THAT WAS ALREADY STARTED|Who worked on this task|The branch",
            ),
        ]));
        let path = world.home.join("agents.toml");
        let config = std::fs::read_to_string(&path)
            .unwrap()
            .replace(
                "[agents.deep-1]\ncli = \"claude\"",
                "[agents.deep-1]\ncli = \"opencode\"\nmodel = \"deepinfra/example\"",
            )
            .replace(
                "[binaries]",
                &format!("[binaries]\nopencode = \"{}\"", script("team-cli.sh").display()),
            );
        std::fs::write(path, config).unwrap();
        configure(
            &world,
            "\n[log]\nlevel = \"debug\"\n[replacement]\nagents = [\"claude-3\"]\n",
        );
        world
    }

    #[test]
    fn refusal_member_retry_changes_provider_in_place_with_briefing_and_audit() {
        use agent_core::state::{RunEnd, RunState, State};
        use agent_core::status::RunOutcome;

        let mut world = refused_member_world();
        world.start();
        let list = world.repo.join("list.md");
        std::fs::write(&list, LIST).unwrap();
        let started = world.agent(&["run", "deep-1", "inv-opus", "claude-2", "-f", list.to_str().unwrap()]);
        assert!(started.status.success(), "{}", stderr(&started));
        let task = stdout(&started)
            .lines()
            .find(|line| line.contains("member") && line.contains("deep-1"))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();
        world.status_where("the first member refusal to pause for its retry", |status| {
            line_of(status, &task).contains("paused")
        });
        assert_eq!(world.runs(&task), 1);
        assert_eq!(log_rows(&world, "task_agent_changed", &task), 0);
        let task_id = task.parse().unwrap();
        let before = State::from_json(&std::fs::read_to_string(world.home.join("state.json")).unwrap()).unwrap();
        let old = before.find_task(&task_id).unwrap().member;
        let worktree = world.home.join("worktrees/example").join(&task);
        let branch = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(branch.status.success());
        std::fs::write(worktree.join("kept-from-refused-run.txt"), "keep me\n").unwrap();

        move_the_refusal_and_the_pause_back(&mut world, "deep-1");
        world.status_where("the replacement to finish the member item", |status| {
            let line = line_of(status, &task);
            line.contains("claude-3") && (line.contains("waiting") || line.contains("done"))
        });

        let after = State::from_json(&std::fs::read_to_string(world.home.join("state.json")).unwrap()).unwrap();
        let member = after.find_task(&task_id).unwrap().member;
        assert_eq!(member.task(), old.task());
        assert_eq!(member.bus_agent(), old.bus_agent());
        assert_eq!(member.past_holders().len(), 1);
        assert_eq!(member.runs().len(), 3);
        assert!(matches!(
            member.runs()[0].state(),
            RunState::Ended {
                end: RunEnd::RateLimited,
                ..
            }
        ));
        assert!(matches!(
            member.runs()[1].state(),
            RunState::Ended {
                end: RunEnd::RateLimited,
                ..
            }
        ));
        assert!(matches!(
            member.runs()[2].state(),
            RunState::Ended {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                },
                ..
            }
        ));
        let current = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(current.status.success());
        assert_eq!(current.stdout, branch.stdout);
        assert_eq!(
            std::fs::read_to_string(worktree.join("kept-from-refused-run.txt")).unwrap(),
            "keep me\n"
        );
        let briefing = std::fs::read_to_string(worktree.join(".fake-prompt-3")).unwrap();
        assert!(briefing.contains("A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"));
        assert!(briefing.contains("deep-1"));
        assert_eq!(log_rows(&world, "task_agent_changed", &task), 1);
        let log = world.agent(&["log", "--event", "decision_taken", "--task", &task, "--json"]);
        assert!(log.status.success(), "{}", stderr(&log));
        let decisions: Vec<serde_json::Value> = stdout(&log)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            decisions
                .iter()
                .filter(|row| row.to_string().contains("takeover"))
                .count(),
            1
        );
    }
}
