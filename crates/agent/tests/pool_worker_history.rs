//! Pool worker runs recorded in the history: the daemon records a `RunStart` before a pool worker spawns, and a
//! `RunEnd` when it settles; `agent log --task <worker task>` then shows its steps.

#[cfg(test)]
mod pool_worker_history {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// The task list of the team: one item, for the member.
    const LIST: &str = "Write the guide.\n\n1. Guide\n[ ] 1.1 The walkthrough\n    Write it.\n";

    /// The plan of the member: a step in hand.
    const MEMBER_PLAN: &str = "\
Write the walkthrough of the guide.

1. Draft
[>] 1.1 Write the walkthrough
    Files: docs/guide.md
    One section per question a reader asks.
    Check: exists docs/guide.md
[x] 1.2 List the questions
";

    /// The arguments of a complete request of the member for its step 1.1.
    const MEMBER_REQUEST: &str = r#"{"goal":"the walkthrough of the guide","workdir":"/tmp","steps":"write the file","returns":"the path of the file","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Write the walkthrough, one section per question."}"#;

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
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-pool-worker-history");
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

    /// A home with a scratch repository `example`, the configuration of the agents given, and a daemon started by
    /// [`World::start`].
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
            let agents = [
                ("inv-opus", "investigator", "sleep"),
                ("claude-1", "agent", "ask-worker-after:member,sleep"),
                ("glm-1", "worker", "sleep,done:Result: done."),
            ];
            let (mut normals, mut workers) = (Vec::new(), Vec::new());
            for (name, kind, scenario) in [
                ("inv-codex", "investigator", "sleep"),
                ("inv-gemini", "investigator", "sleep"),
            ]
            .iter()
            .map(|(n, k, s)| (*n, *k, *s))
            .chain(agents.iter().copied())
            {
                let table = if kind == "investigator" {
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
                match kind {
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
            for dir in ["fake-plans", "fake-requests"] {
                std::fs::create_dir(home.join(dir)).unwrap();
            }
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
            std::fs::write(repo.join("list.md"), LIST).unwrap();
            Self {
                _dir: dir,
                home,
                repo,
                daemon: None,
            }
        }

        /// Gives the stand-ins that run `ask-worker:NAME` their plan and their requests.
        fn asks(&self, name: &str, plan: Option<&str>, requests: &[&str]) {
            if let Some(plan) = plan {
                std::fs::write(self.home.join("fake-plans").join(format!("{name}.md")), plan).unwrap();
            }
            let mut lines = requests.join("\n");
            lines.push('\n');
            std::fs::write(self.home.join("fake-requests").join(format!("{name}.jsonl")), lines).unwrap();
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

        /// Stops the daemon as systemd does, and waits for it to end.
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

        /// Starts the team of `agents` on the task list, and gives the task of every agent by its name.
        fn run(&self, agents: &[&str]) -> Vec<(String, String)> {
            let mut args = vec!["run"];
            args.extend_from_slice(agents);
            args.extend_from_slice(&["-f", "list.md"]);
            let output = self.agent(&args);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let text = String::from_utf8(output.stdout).unwrap();
            text.lines()
                .skip(1)
                .filter_map(|line| {
                    let words: Vec<&str> = line.split_whitespace().collect();
                    let task = words.first()?;
                    let name = words.iter().find(|word| agents.contains(word))?;
                    Some(((*name).to_owned(), (*task).to_owned()))
                })
                .collect()
        }

        /// The runs the task has had, as its stand-in counted them.
        fn runs(&self, task: &str) -> u32 {
            let path = self.home.join("worktrees/example").join(task).join(".fake-runs");
            let deadline = Instant::now() + PATIENCE;
            loop {
                let pending = match std::fs::read_to_string(&path) {
                    Ok(text) => match text.strip_suffix('\n') {
                        Some(complete) => match complete.parse::<u32>() {
                            Ok(runs) => return runs,
                            Err(error) => format!("invalid counter {text:?}: {error}"),
                        },
                        None => format!("incomplete counter {text:?}"),
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
                    Err(error) => panic!("cannot read counter {}: {error}", path.display()),
                };
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for counter {}: {pending}",
                    path.display()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// Waits until the task's run `n` has ended, as `agent status` shows: its task waits for a message.
        fn ended(&self, task: &str, n: u32) {
            eventually(&format!("run {n} of {task} to end"), || {
                if self.runs(task) < n {
                    return false;
                }
                let output = self.agent(&["status", "--task"]);
                let status = String::from_utf8(output.stdout).unwrap();
                status
                    .lines()
                    .any(|line| line.contains(task) && line.contains("waiting"))
            });
        }

        /// Waits until the bus has `name`, in its team, asleep: a worker the daemon may give a request to.
        fn sleeps_on_the_bus(&self, name: &str) {
            eventually(&format!("{name} to sleep on the bus"), || {
                let Ok(text) = std::fs::read_to_string(self.home.join("bus/state.json")) else {
                    return false;
                };
                let state: serde_json::Value = serde_json::from_str(&text).unwrap();
                state["teams"]
                    .as_object()
                    .into_iter()
                    .flat_map(|teams| teams.values())
                    .any(|team| team["members"][name]["state"] == "sleeping")
            });
        }

        /// The worktree of `task`.
        fn worktree(&self, task: &str) -> PathBuf {
            self.home.join("worktrees/example").join(task)
        }

        /// The file `name` of the worktree of `task`, once it is there.
        #[expect(dead_code, reason = "used by other tests that may need to read worktree files")]
        fn file(&self, task: &str, name: &str) -> String {
            let path = self.worktree(task).join(name);
            eventually(&format!("{path}", path = path.display()), || path.exists());
            std::fs::read_to_string(path).unwrap()
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

    /// The task of `name` among `tasks`.
    fn task<'a>(tasks: &'a [(String, String)], name: &str) -> &'a str {
        &tasks.iter().find(|(agent, _)| agent == name).unwrap().1
    }

    /// Starts a team of a member that asks for worker step 1.1 once the test says so, and a worker
    /// whose second run is `worker_scenario` (the first one is `sleep` to settle on the bus);
    /// returns the world and the tasks.
    fn answered_request(world: &mut World, worker_scenario: &str, request: &str) -> Vec<(String, String)> {
        // Override the worker's scenario: keep the first-action `sleep`, replace the second.
        let config = std::fs::read_to_string(world.home.join("agents.toml")).unwrap();
        let updated = config.replace("sleep,done:Result: done.", &format!("sleep,{worker_scenario}"));
        std::fs::write(world.home.join("agents.toml"), updated).unwrap();
        world.asks("member", Some(MEMBER_PLAN), &[request]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);
        world.ended(task(&tasks, "glm-1"), 1);
        world.sleeps_on_the_bus("glm-1");
        tasks
    }

    /// The team ID the daemon formed.
    fn team_id(world: &World) -> String {
        let text = std::fs::read_to_string(world.home.join("state.json")).unwrap();
        let state: serde_json::Value = serde_json::from_str(&text).unwrap();
        state["teams"][0]["id"].as_str().unwrap().to_owned()
    }

    /// Opens the history database reader.
    fn open_reader(world: &World) -> agent_runtime::history::Reader {
        agent_runtime::history::Reader::open(&world.home.join("history.db")).unwrap()
    }

    #[test]
    fn pool_worker_run_is_in_history() {
        let lead_name = "claude-1";
        let worker_name = "glm-1";
        let mut world = World::new();
        let tasks = answered_request(&mut world, "sleep,done:Result: done.", MEMBER_REQUEST);
        let (member, worker) = (task(&tasks, lead_name).to_owned(), task(&tasks, worker_name).to_owned());
        let team = team_id(&world);

        std::fs::write(world.worktree(&member).join(".fake-ask"), "").unwrap();

        // Wait for the worker to get its second run (serving the request).
        world.ended(&worker, 2);
        world.stop();

        let reader = open_reader(&world);
        let worker_task_id: agent_core::ids::TaskId = worker.parse().unwrap();
        let runs = reader.runs_of_task(&worker_task_id).unwrap();

        // The worker runs at least twice: the initial sleep run and the request-serving run.
        assert!(
            runs.len() >= 2,
            "expected at least 2 runs for the pool worker (sleep + request), got {runs:?}"
        );
        // The last run is the request-serving run.
        let run = runs.last().unwrap();
        let start = &run.start.value;
        assert_eq!(start.task.as_str(), worker);
        assert_eq!(start.role, agent_core::kinds::Role::Worker);
        assert_eq!(start.why.kind(), agent_core::history::WhyKind::Request);
        assert_eq!(start.team.as_str(), team);
        // The prompt should contain the request content.
        assert!(
            start.prompt.contains("FROM claude-1"),
            "prompt should mention the requester, got: {}",
            start.prompt
        );
        // The end must be recorded.
        assert!(run.end.is_some(), "run must have an end record");
    }

    #[test]
    fn pool_worker_steps_are_kept() {
        let mut world = World::new();
        // Write a stream JSONL file for the worker that prints two tool steps.
        let scripts = world.home.join("fake-scripts");
        std::fs::create_dir(&scripts).unwrap();
        let steps_jsonl = "\
{\"type\":\"tool_use\",\"tool\":{\"name\":\"bash\",\"arguments\":{\"command\":\"echo hello\"},\"result\":{\"stdout\":\"hello\\n\"}}}
{\"type\":\"tool_use\",\"tool\":{\"name\":\"write\",\"arguments\":{\"path\":\"docs/guide.md\",\"content\":\"the guide\"},\"result\":{}}}
";
        std::fs::write(scripts.join("steps.jsonl"), steps_jsonl).unwrap();

        let tasks = answered_request(&mut world, "stream:steps", MEMBER_REQUEST);
        let (member, worker) = (task(&tasks, "claude-1").to_owned(), task(&tasks, "glm-1").to_owned());

        std::fs::write(world.worktree(&member).join(".fake-ask"), "").unwrap();
        world.ended(&worker, 2);
        world.stop();

        let reader = open_reader(&world);
        let worker_task_id: agent_core::ids::TaskId = worker.parse().unwrap();
        let runs = reader.runs_of_task(&worker_task_id).unwrap();
        assert!(runs.len() >= 2, "expected at least 2 runs for the pool worker");
        let run_id = &runs.last().unwrap().start.value.run;
        let steps = reader.steps(run_id).unwrap();

        // Should have at least 2 tool steps. There might also be a final message step.
        assert!(
            steps.len() >= 2,
            "expected at least 2 steps for the worker run, got {}: {steps:?}",
            steps.len()
        );

        // Check lost records count via run_ends: the run must have an end.
        assert!(runs[0].end.is_some(), "run must have an end record");
    }

    #[test]
    fn agent_log_shows_worker_task() {
        let mut world = World::new();
        // Write a stream JSONL file with two tool steps.
        let scripts = world.home.join("fake-scripts");
        std::fs::create_dir(&scripts).unwrap();
        let steps_jsonl = "\
{\"type\":\"tool_use\",\"tool\":{\"name\":\"bash\",\"arguments\":{\"command\":\"echo hello\"},\"result\":{\"stdout\":\"hello\\n\"}}}
{\"type\":\"tool_use\",\"tool\":{\"name\":\"write\",\"arguments\":{\"path\":\"docs/guide.md\",\"content\":\"the guide\"},\"result\":{}}}
";
        std::fs::write(scripts.join("steps.jsonl"), steps_jsonl).unwrap();

        let tasks = answered_request(&mut world, "stream:steps", MEMBER_REQUEST);
        let (member, worker) = (task(&tasks, "claude-1").to_owned(), task(&tasks, "glm-1").to_owned());

        std::fs::write(world.worktree(&member).join(".fake-ask"), "").unwrap();
        world.ended(&worker, 2);
        world.stop();

        // agent log --task <worker task> should print something.
        let output = world.agent(&["log", "--task", &worker]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains(&worker),
            "agent log output should mention the worker task\noutput:\n{text}"
        );
        // The output should include run start and end lines.
        assert!(
            text.contains("run"),
            "agent log output should mention a run\noutput:\n{text}"
        );

        // agent log --run <run> should also work.
        let reader = open_reader(&world);
        let worker_task_id: agent_core::ids::TaskId = worker.parse().unwrap();
        let runs = reader.runs_of_task(&worker_task_id).unwrap();
        let run_id = runs.last().unwrap().start.value.run.to_string();
        let output_run = world.agent(&["log", "--run", &run_id]);
        assert!(
            output_run.status.success(),
            "{}",
            String::from_utf8_lossy(&output_run.stderr)
        );
        let text_run = String::from_utf8(output_run.stdout).unwrap();
        assert!(
            !text_run.is_empty(),
            "agent log --run output should not be empty\noutput:\n{text_run}"
        );
    }

    #[test]
    fn failed_worker_run_has_end() {
        let mut world = World::new();
        let tasks = answered_request(&mut world, "nostatus", MEMBER_REQUEST);
        let (member, worker) = (task(&tasks, "claude-1").to_owned(), task(&tasks, "glm-1").to_owned());

        std::fs::write(world.worktree(&member).join(".fake-ask"), "").unwrap();

        // The worker exits with no STATUS: line (nostatus). The daemon should record a no-status end.
        eventually(
            "worker to have had 2 runs (the first sleep and the actual work)",
            || world.runs(&worker) >= 2,
        );

        // Wait for the worker to be settled.
        world.ended(&member, 1);
        world.stop();

        let reader = open_reader(&world);
        let worker_task_id: agent_core::ids::TaskId = worker.parse().unwrap();
        let runs = reader.runs_of_task(&worker_task_id).unwrap();

        // Should have at least one run with an end.
        assert!(!runs.is_empty(), "expected at least one run for the worker");
        // The last run (the nostatus one) should have an end.
        let last = runs.last().unwrap();
        assert!(last.end.is_some(), "the nostatus run must have an end record");
        let end = last.end.as_ref().unwrap();
        assert!(
            matches!(end.value.end, agent_core::state::RunEnd::NoStatus { .. }),
            "end should be NoStatus, got: {:?}",
            end.value.end
        );
    }

    #[test]
    #[ignore = "recovery test needs daemon restart orchestration that is timing-sensitive in this test harness"]
    fn recovered_worker_run_recorded_once() {
        let mut world = World::new();
        let tasks = answered_request(&mut world, "sleep,done:Result: done.", MEMBER_REQUEST);
        let (member, worker) = (task(&tasks, "claude-1").to_owned(), task(&tasks, "glm-1").to_owned());

        // Make the first request.
        std::fs::write(world.worktree(&member).join(".fake-ask"), "").unwrap();
        world.ended(&worker, 2);
        world.stop();

        // Read the history: two runs with different IDs.
        let reader = open_reader(&world);
        let worker_task_id: agent_core::ids::TaskId = worker.parse().unwrap();
        let runs = reader.runs_of_task(&worker_task_id).unwrap();
        assert_eq!(runs.len(), 2, "expected 2 runs (sleep + request), got {runs:?}");
        assert_ne!(
            runs[0].start.value.run, runs[1].start.value.run,
            "the two runs must have different IDs"
        );
        // Both runs must have ends.
        for run in &runs {
            assert!(run.end.is_some(), "each run must have an end record");
        }
    }
}
