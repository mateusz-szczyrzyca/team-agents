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
        /// Gives the stand-ins that run `script:NAME` their script: it runs in the worktree of the agent.
        fn script(&self, name: &str, body: &str) {
            let dir = self.home.join("fake-scripts");
            std::fs::create_dir_all(&dir).unwrap();
            let exclude = "printf '%s\\n' '.fake-*' >> \"$(git rev-parse --git-path info/exclude)\"\n";
            std::fs::write(dir.join(format!("{name}.sh")), format!("{exclude}{body}")).unwrap();
        }

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

        /// The worker requests the bus keeps, by number.
        fn requests(&self) -> serde_json::Map<String, serde_json::Value> {
            let text = std::fs::read_to_string(self.home.join("bus/state.json")).unwrap();
            let state: serde_json::Value = serde_json::from_str(&text).unwrap();
            state
                .get("helpers")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default()
        }

        /// The task of the pool worker that serves request `number`, once the request has one.
        fn helper_task(&self, number: &str) -> Option<String> {
            self.requests()
                .get(number)
                .and_then(|request| request["helper_task"].as_str())
                .map(str::to_owned)
        }

        /// Waits until request `number` is no longer open: the worker's answer went to the requester.
        fn request_settled(&self, number: &str) {
            eventually(&format!("request H{number} to be settled"), || {
                self.requests()
                    .get(number)
                    .is_some_and(|request| matches!(request["status"].as_str(), Some("done" | "failed" | "unclear")))
            });
        }

        /// The worktree of `task`.
        fn worktree(&self, task: &str) -> PathBuf {
            self.home.join("worktrees/example").join(task)
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

    /// The script of a pool worker: it commits the file its request names and prints two tool steps, as the stream
    /// of Claude Code does.
    const WORKER_SCRIPT: &str = r#"mkdir -p docs
printf 'the walkthrough\n' > docs/guide.md
git add docs/guide.md
git -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"echo hello"}}]}}' '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"docs/guide.md","content":"the guide"}}]}}'
"#;

    /// Starts a team of a member that asks for its step 1.1 once the test says so; the worker is not in the team, so
    /// the request goes to a worker of the pool, whose run is `worker_scenario`. Returns the member's task and the
    /// worker's task, once the request is settled.
    fn serve_request(world: &mut World, worker_scenario: &str) -> (String, String) {
        let member = ask_for_worker(world, worker_scenario);
        world.request_settled("1");
        let worker = world.helper_task("1").unwrap();
        (member, worker)
    }

    /// Starts the daemon and the team of the member, and lets the member ask for a worker whose run is
    /// `worker_scenario`. Returns the member's task.
    fn ask_for_worker(world: &mut World, worker_scenario: &str) -> String {
        let config = std::fs::read_to_string(world.home.join("agents.toml")).unwrap();
        let updated = config.replace("sleep,done:Result: done.", worker_scenario);
        std::fs::write(world.home.join("agents.toml"), updated).unwrap();
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1").to_owned();
        let worktree = world.worktree(&member);
        eventually("the worktree of the member", || worktree.exists());
        std::fs::write(worktree.join(".fake-ask"), "").unwrap();
        member
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

    /// The rows of the runs of `worker` that serve a request.
    fn request_runs(reader: &agent_runtime::history::Reader, worker: &str) -> Vec<agent_runtime::history::RunRow> {
        let task: agent_core::ids::TaskId = worker.parse().unwrap();
        reader
            .runs_of_task(&task)
            .unwrap()
            .into_iter()
            .filter(|row| row.start.value.why.kind() == agent_core::history::WhyKind::Request)
            .collect()
    }

    #[test]
    fn pool_worker_run_is_in_history() {
        let mut world = World::new();
        world.script("guide", WORKER_SCRIPT);
        let (_member, worker) = serve_request(&mut world, "script:guide");
        let team = team_id(&world);
        world.stop();

        let reader = open_reader(&world);
        let all_runs = reader.runs_of_task(&worker.parse().unwrap()).unwrap();
        let runs = request_runs(&reader, &worker);
        assert_eq!(runs.len(), 1, "expected one run for the pool worker, got {all_runs:?}");
        assert_eq!(all_runs.len(), 1, "the pool worker has no other run: {all_runs:?}");
        let run = &runs[0];
        let start = &run.start.value;
        assert_eq!(start.task.as_str(), worker);
        assert_eq!(start.role, agent_core::kinds::Role::Worker);
        assert_eq!(start.agent.tier, agent_core::kinds::Tier::Worker);
        assert_eq!(start.team.as_str(), team);
        assert!(
            matches!(start.why, agent_core::history::RunWhy::Request { asked_by: Some(_) }),
            "{:?}",
            start.why
        );
        assert_eq!(start.session, None);
        assert!(
            start.prompt.contains("one section per question"),
            "prompt should contain the request text, got: {}",
            start.prompt
        );
        let end = run.end.as_ref().expect("the run has an end");
        assert!(
            matches!(end.value.end, agent_core::state::RunEnd::Reported { .. }),
            "{:?}",
            end.value.end
        );
    }

    #[test]
    fn pool_worker_steps_are_kept() {
        let mut world = World::new();
        world.script("guide", WORKER_SCRIPT);
        let (_member, worker) = serve_request(&mut world, "script:guide");
        let status = String::from_utf8(world.agent(&["status"]).stdout).unwrap();
        assert!(!status.contains("lost"), "no record is lost: {status}");
        world.stop();

        let reader = open_reader(&world);
        let runs = request_runs(&reader, &worker);
        assert_eq!(runs.len(), 1, "expected exactly 1 request-serving run");
        let steps = reader.steps(&runs[0].start.value.run).unwrap();
        let tool_steps: Vec<_> = steps
            .iter()
            .filter(|s| {
                matches!(
                    agent_core::history::StepKind::of(&s.value.event),
                    agent_core::history::StepKind::Command
                        | agent_core::history::StepKind::FileEdit
                        | agent_core::history::StepKind::ToolCall
                )
            })
            .collect();
        assert_eq!(
            tool_steps.len(),
            2,
            "expected exactly 2 tool steps for the worker run, got: {tool_steps:?}"
        );
    }

    #[test]
    fn agent_log_shows_worker_task() {
        let mut world = World::new();
        world.script("guide", WORKER_SCRIPT);
        let (_member, worker) = serve_request(&mut world, "script:guide");
        world.stop();

        let output = world.agent(&["log", "--task", &worker]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains("started by"),
            "agent log --task should show the start of the run\noutput:\n{text}"
        );
        assert!(
            text.contains("echo hello") && text.contains("docs/guide.md"),
            "agent log --task should show both steps\noutput:\n{text}"
        );

        let reader = open_reader(&world);
        let runs = request_runs(&reader, &worker);
        let run_id = runs[0].start.value.run.to_string();
        let output_run = world.agent(&["log", "--run", &run_id]);
        assert!(
            output_run.status.success(),
            "{}",
            String::from_utf8_lossy(&output_run.stderr)
        );
        let text_run = String::from_utf8(output_run.stdout).unwrap();
        assert!(
            text_run.contains("echo hello") && text_run.contains("docs/guide.md"),
            "agent log --run should show both steps\noutput:\n{text_run}"
        );
    }

    #[test]
    fn failed_worker_run_has_end() {
        let mut world = World::new();
        let (_member, worker) = serve_request(&mut world, "nostatus");
        world.stop();

        let reader = open_reader(&world);
        let runs = request_runs(&reader, &worker);
        assert!(!runs.is_empty(), "expected at least one run for the worker");
        for run in &runs {
            let end = run.end.as_ref().expect("the run has an end");
            assert!(
                matches!(end.value.end, agent_core::state::RunEnd::NoStatus { .. }),
                "end should be NoStatus, got: {:?}",
                end.value.end
            );
        }
    }

    #[test]
    fn recovered_worker_run_recorded_once() {
        let mut world = World::new();
        world.script(
            "durable-restart",
            r#"
if test ! -f .fake-started; then
    touch .fake-started
    while test ! -f "$AGENTS_HOME/release-worker"; do sleep 0.05; done
fi
mkdir -p docs
printf 'the walkthrough\n' > docs/guide.md
git add docs/guide.md
git -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'
"#,
        );
        let member = ask_for_worker(&mut world, "script:durable-restart");
        eventually("the worker to run on the request", || {
            world
                .helper_task("1")
                .is_some_and(|task| world.worktree(&task).join(".fake-started").exists())
        });
        let worker = world.helper_task("1").unwrap();

        // Stop the daemon with the durable files as they were while the request ran: the crash window.
        let journal_path = world.home.join("tasks").join(&member).join("worker-recovery-1.json");
        let running_journal = std::fs::read(&journal_path).unwrap();
        let running_bus = std::fs::read(world.home.join("bus/state.json")).unwrap();
        world.stop();
        std::fs::write(&journal_path, running_journal).unwrap();
        std::fs::write(world.home.join("bus/state.json"), running_bus).unwrap();

        std::fs::write(world.home.join("release-worker"), "").unwrap();
        world.start();
        world.request_settled("1");
        world.stop();

        let reader = open_reader(&world);
        let all_runs = reader.runs_of_task(&worker.parse().unwrap()).unwrap();
        let runs = request_runs(&reader, &worker);
        assert_eq!(
            runs.len(),
            2,
            "the request is given out again after the restart: two request runs, got {all_runs:?}"
        );
        assert_ne!(
            runs[0].start.value.run, runs[1].start.value.run,
            "the two request runs must have different run IDs"
        );
        let ids: std::collections::HashSet<_> = all_runs.iter().map(|row| row.start.value.run.clone()).collect();
        assert_eq!(ids.len(), all_runs.len(), "each run has exactly one row: {all_runs:?}");
        let last_end = runs[1].end.as_ref().expect("the run after the restart has an end");
        assert!(
            matches!(last_end.value.end, agent_core::state::RunEnd::Reported { .. }),
            "{:?}",
            last_end.value.end
        );
    }
}
