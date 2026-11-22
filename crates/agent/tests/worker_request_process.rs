//! A request to a worker from an agent with a plan, through the whole harness: `agent daemon` in a temporary
//! `AGENTS_HOME`, the stand-in CLI `team-cli.sh` for every agent and the real `agentbus`. A run asks for workers through
//! the agentbus server of the MCP configuration the daemon wrote for it (the action `ask-worker:NAME` of the stand-in),
//! so these tests see what an agent sees: the bus's answer to `delegate`, whether a worker starts at all, and the
//! prompt the worker is given.

#[cfg(test)]
mod worker_request_process {
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

    /// The plan of the member: a step in hand, a step done.
    const MEMBER_PLAN: &str = "\
Write the walkthrough of the guide.

1. Draft
[>] 1.1 Write the walkthrough
    Files: docs/guide.md
    One section per question a reader asks.
    Check: exists docs/guide.md
[x] 1.2 List the questions
";

    /// The plan of the coordinator: one step to do.
    const COORDINATOR_PLAN: &str = "\
Integrate the guide.

1. Integrate
[ ] 1.1 Count the sections of the guide
    Files: docs/guide.md
    Check: test -s docs/count.txt
";

    /// The arguments of a complete request of the member for its step 1.1.
    const MEMBER_REQUEST: &str = r#"{"goal":"the walkthrough of the guide","workdir":"/tmp","steps":"write the file","returns":"the path of the file","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Write the walkthrough, one section per question."}"#;

    /// The arguments of a complete request of the coordinator for its step 1.1.
    const COORDINATOR_REQUEST: &str = r#"{"goal":"the count of the sections","workdir":"/tmp","steps":"count the headings","returns":"the number","for":"1.1","files":"docs/guide.md, docs/count.txt","check":"test -s docs/count.txt","do":"Count the level-two headings and write the number to docs/count.txt."}"#;

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
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-worker-requests");
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

    /// `(name, kind, scenario)` of an agent: kind `agent`, `worker` or `investigator`.
    type AgentSpec = (&'static str, &'static str, &'static str);

    /// The two investigators outside the team that its reviews need.
    const REVIEWERS: [AgentSpec; 2] = [
        ("inv-codex", "investigator", "sleep"),
        ("inv-gemini", "investigator", "sleep"),
    ];

    /// A home with a scratch repository `example` (its origin is `https://example.com/org/example.git`), the
    /// configuration of the agents given, the plans and requests of the stand-ins, and a daemon started by
    /// [`World::start`].
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        daemon: Option<Child>,
    }

    impl World {
        fn new(agents: &[AgentSpec]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            let (mut normals, mut workers) = (Vec::new(), Vec::new());
            for (name, kind, scenario) in REVIEWERS.iter().chain(agents) {
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

        /// Gives the stand-ins that run `ask-worker:NAME` their plan (`None`: no plan file) and their requests.
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

        /// The worktree of `task`.
        fn worktree(&self, task: &str) -> PathBuf {
            self.home.join("worktrees/example").join(task)
        }

        /// The file `name` of the worktree of `task`, once it is there.
        fn file(&self, task: &str, name: &str) -> String {
            let path = self.worktree(task).join(name);
            eventually(&format!("{}", path.display()), || path.exists());
            std::fs::read_to_string(path).unwrap()
        }

        /// The runs the task has had, as its stand-in counted them.
        fn runs(&self, task: &str) -> u32 {
            std::fs::read_to_string(self.worktree(task).join(".fake-runs"))
                .map_or(0, |text| text.trim().parse().unwrap())
        }

        /// The bus's worker requests, by their ID.
        fn requests(&self) -> serde_json::Map<String, serde_json::Value> {
            let text = std::fs::read_to_string(self.home.join("bus/state.json")).unwrap();
            let state: serde_json::Value = serde_json::from_str(&text).unwrap();
            state
                .get("helpers")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default()
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
    }

    impl World {
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

        /// Calls `delegate` with `arguments` as the agent of `task`'s last run, through the agentbus server of the MCP
        /// configuration the daemon wrote for that run, with `agent` of this build first on the `PATH` as in a run.
        fn delegate_as(&self, task: &str, arguments: &str) -> String {
            let runs = self.home.join("tasks").join(task).join("runs");
            let mut configs: Vec<PathBuf> = std::fs::read_dir(&runs)
                .unwrap()
                .map(|entry| entry.unwrap().path().join("mcp.json"))
                .filter(|path| path.exists())
                .collect();
            configs.sort();
            let config: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(configs.last().unwrap()).unwrap()).unwrap();
            let server = &config["mcpServers"]["agentbus"];
            let args: Vec<&str> = server["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap())
                .collect();
            let agent_dir = Path::new(env!("CARGO_BIN_EXE_agent")).parent().unwrap();
            let path = format!("{}:{}", agent_dir.display(), std::env::var("PATH").unwrap());
            let mut bus = Command::new(server["command"].as_str().unwrap())
                .args(&args)
                .env("PATH", path)
                .env("AGENTS_HOME", &self.home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let call = format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"delegate\",\"arguments\":{arguments}}}}}\n"
            );
            std::io::Write::write_all(&mut bus.stdin.take().unwrap(), call.as_bytes()).unwrap();
            let out = bus.wait_with_output().unwrap();
            String::from_utf8(out.stdout).unwrap()
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

    /// The bus's answers in a `.fake-delegate-N` file: for every call, whether it was refused and its text.
    fn answers(text: &str) -> Vec<(bool, String)> {
        text.lines()
            .map(|line| {
                let reply: serde_json::Value = serde_json::from_str(line).unwrap();
                let result = &reply["result"];
                (
                    result["isError"].as_bool().unwrap(),
                    result["content"][0]["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// The prompts of `task`'s runs `from..=to` that hold a request to a worker.
    fn request_prompts(world: &World, task: &str, from: u32, to: u32) -> Vec<String> {
        (from..=to)
            .map(|n| world.file(task, &format!(".fake-prompt-{n}")))
            .filter(|prompt| prompt.contains("A REQUEST FROM"))
            .collect()
    }

    #[test]
    fn a_complete_request_of_a_member_and_of_an_investigator_each_start_a_worker_whose_prompt_holds_the_step() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "ask-worker-after:coordinator,sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "ask-worker:worker,done:Result: done."),
            ("glm-2", "worker", "sleep,done:Result: done."),
        ]);
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.asks("coordinator", Some(COORDINATOR_PLAN), &[COORDINATOR_REQUEST]);
        world.asks(
            "worker",
            None,
            &[r#"{"goal":"a look-up","workdir":"/tmp","steps":"ls","returns":"the list"}"#],
        );
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus", "glm-1", "glm-2"]);

        let (glm_1, glm_2) = (task(&tasks, "glm-1"), task(&tasks, "glm-2"));
        // The requesters ask once both workers wait: only a waiting worker is given a request.
        for name in ["glm-1", "glm-2"] {
            world.ended(task(&tasks, name), 1);
            world.sleeps_on_the_bus(name);
        }
        for name in ["claude-1", "inv-opus"] {
            std::fs::write(world.worktree(task(&tasks, name)).join(".fake-ask"), "").unwrap();
        }
        for name in ["claude-1", "inv-opus", "glm-1"] {
            world.ended(task(&tasks, name), 1);
        }
        let member = answers(&world.file(task(&tasks, "claude-1"), ".fake-delegate-1"));
        let coordinator = answers(&world.file(task(&tasks, "inv-opus"), ".fake-delegate-1"));
        let worker = answers(&world.file(glm_1, ".fake-delegate-1"));
        assert!(!member[0].0 && member[0].1.contains("accepted"), "{member:?}");
        assert!(
            !coordinator[0].0 && coordinator[0].1.contains("accepted"),
            "{coordinator:?}"
        );
        assert!(worker[0].0, "a worker does not delegate: {worker:?}");
        eventually("both requests to be done", || {
            let requests = world.requests();
            requests.len() == 2
                && requests
                    .values()
                    .all(|request| request["status"] == "done" || request["status"] == "failed")
        });
        let prompts = [
            request_prompts(&world, glm_1, 1, world.runs(glm_1)),
            request_prompts(&world, glm_2, 1, world.runs(glm_2)),
        ]
        .concat();
        assert_eq!(prompts.len(), 2, "{prompts:?}");
        let from_member = prompts.iter().find(|p| p.contains("FROM claude-1")).unwrap();
        for line in [
            "The step of the requester's plan:",
            "Step 1.1 Write the walkthrough",
            "One section per question a reader asks.",
            "Files: docs/guide.md",
            "Check: exists docs/guide.md",
            "Request files: docs/guide.md",
            "Request check: exists docs/guide.md",
            "Do: Write the walkthrough, one section per question.",
        ] {
            assert!(from_member.contains(line), "{line:?} is not in:\n{from_member}");
        }
        let from_coordinator = prompts.iter().find(|p| p.contains("FROM inv-opus")).unwrap();
        for line in [
            "Step 1.1 Count the sections of the guide",
            "Request files: docs/guide.md, docs/count.txt",
            "Request check: test -s docs/count.txt",
            "Do: Count the level-two headings and write the number to docs/count.txt.",
        ] {
            assert!(
                from_coordinator.contains(line),
                "{line:?} is not in:\n{from_coordinator}"
            );
        }
        assert_eq!(world.requests().len(), 2, "the worker's own call queued nothing");
    }

    #[test]
    fn a_request_that_leaves_something_to_guess_is_refused_and_no_worker_starts() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker:refusals,sleep"),
            ("glm-1", "worker", "sleep,done:Result: done."),
        ]);
        let with = |field: &str, value: &str| {
            let mut request: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
            request[field] = serde_json::Value::String(value.to_owned());
            request.to_string()
        };
        let cases = [
            (with("for", ""), "names no step of your plan"),
            (with("for", "9.9"), "names no step of your plan"),
            (with("for", "1.2"), "is done"),
            (with("files", ""), "names no files"),
            (with("files", "../secret.txt"), "no path of the repository"),
            (with("check", ""), "names no check"),
            (with("check", "agent gate"), "never the repository's gate"),
            (with("check", "make gate"), "never the repository's gate"),
            (with("check", "reviewer"), "never reviewer"),
            (with("do", ""), "nothing to do"),
        ];
        let requests: Vec<&str> = cases.iter().map(|(request, _)| request.as_str()).collect();
        world.asks("refusals", Some(MEMBER_PLAN), &requests);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);

        let member = task(&tasks, "claude-1");
        world.ended(member, 1);
        let got = answers(&world.file(member, ".fake-delegate-1"));
        assert_eq!(got.len(), cases.len(), "{got:?}");
        for ((refused, text), (request, why)) in got.iter().zip(&cases) {
            assert!(*refused, "{request} was not refused: {text}");
            assert!(text.contains(why), "{request}: {why:?} is not in {text:?}");
        }
        assert!(world.requests().is_empty(), "{:?}", world.requests());
        assert_eq!(
            world.runs(task(&tasks, "glm-1")),
            1,
            "the worker is not started for a refused request"
        );
    }

    #[test]
    fn a_request_when_no_worker_of_the_team_is_free_is_refused_at_once() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker:member,sleep"),
        ]);
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);

        world.ended(task(&tasks, "claude-1"), 1);
        let got = answers(&world.file(task(&tasks, "claude-1"), ".fake-delegate-1"));
        assert!(got[0].0, "{got:?}");
        assert!(got[0].1.contains("no worker of your team is free"), "{got:?}");
        assert!(world.requests().is_empty(), "{:?}", world.requests());
    }

    #[test]
    fn a_request_queued_before_a_restart_of_the_daemon_is_served_once_after_it() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "sleep"),
            ("glm-1", "worker", "sleep,done:Result: done."),
        ]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);
        let (member, glm_1) = (task(&tasks, "claude-1"), task(&tasks, "glm-1"));
        for name in ["claude-1", "inv-opus", "glm-1"] {
            world.ended(task(&tasks, name), 1);
        }
        world.sleeps_on_the_bus("glm-1");

        // The daemon goes down; the member's request reaches the bus of its run before the daemon is back.
        world.stop();
        std::fs::create_dir_all(world.worktree(member).join(".agent")).unwrap();
        std::fs::write(world.worktree(member).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        let got = answers(&world.delegate_as(member, MEMBER_REQUEST));
        assert!(!got[0].0, "{got:?}");
        assert_eq!(world.requests()["1"]["status"], "queued");
        world.start();

        eventually("the request to be answered", || {
            ["done", "failed"].contains(&world.requests()["1"]["status"].as_str().unwrap())
        });
        let runs = world.runs(glm_1);
        assert_eq!(
            request_prompts(&world, glm_1, 1, runs).len(),
            1,
            "the request is served once"
        );
        assert_eq!(world.requests().len(), 1);
    }

    #[test]
    fn agent_status_of_a_task_shows_its_request_with_its_step_its_worker_and_its_state() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,done:Result: done."),
        ]);
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);
        let member = task(&tasks, "claude-1");
        world.ended(task(&tasks, "glm-1"), 1);
        world.sleeps_on_the_bus("glm-1");
        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();
        eventually("the request to be answered", || {
            world
                .requests()
                .get("1")
                .is_some_and(|request| request["status"] == "failed")
        });

        let output = world.agent(&["status", member]);

        let status = String::from_utf8(output.stdout).unwrap();
        assert!(
            status.contains("    request H1  step 1.1  worker glm-1  failed"),
            "{status}"
        );
    }

    impl World {
        /// Gives the stand-ins that run `script:NAME` their script: it runs in the worktree of the agent.
        fn script(&self, name: &str, body: &str) {
            let dir = self.home.join("fake-scripts");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.sh")), body).unwrap();
        }

        /// What a git command of the worktree of `task` prints, without its last line break.
        fn git_output(&self, task: &str, args: &[&str]) -> String {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.worktree(task))
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
        }

        /// The texts of the messages the bus holds for the agent `name`.
        fn messages_to(&self, name: &str) -> Vec<String> {
            let text = std::fs::read_to_string(self.home.join("bus/messages.jsonl")).unwrap_or_default();
            text.lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|message| message["to"] == name)
                .filter_map(|message| message["text"].as_str().map(str::to_owned))
                .collect()
        }

        /// The text of the message to `name` that holds `needle`, once the bus has it.
        fn told(&self, name: &str, needle: &str) -> String {
            let deadline = Instant::now() + PATIENCE;
            loop {
                let messages = self.messages_to(name);
                if let Some(text) = messages.iter().find(|text| text.contains(needle)) {
                    return text.clone();
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {name} to be told {needle:?}; its messages: {messages:#?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// Waits until the request `number` of the bus has the status `status`.
        fn request_is(&self, number: &str, status: &str) {
            eventually(&format!("request H{number} to be {status}"), || {
                self.requests()
                    .get(number)
                    .is_some_and(|request| request["status"] == status)
            });
        }
    }

    /// A worker's script that commits the file `docs/guide.md` with the text `text`, and the files `more`.
    fn commit_script(text: &str, more: &[&str]) -> String {
        let mut script = format!("mkdir -p docs\nprintf '{text}\\n' > docs/guide.md\ngit add docs/guide.md\n");
        for file in more {
            script.push_str(&format!("printf 'x\\n' > {file}\ngit add {file}\n"));
        }
        script.push_str(
            "git -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'\n",
        );
        script
    }

    /// Starts a team of a member that asks for its step 1.1 with `request` once the test says so, and a worker whose
    /// second run is `scenario`; returns the world and the tasks, with the request answered.
    fn answered_request(world: &mut World, request: &str) -> Vec<(String, String)> {
        world.asks("member", Some(MEMBER_PLAN), &[request]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);
        world.ended(task(&tasks, "glm-1"), 1);
        world.sleeps_on_the_bus("glm-1");
        tasks
    }

    #[test]
    fn a_worker_that_commits_the_requested_file_has_its_commit_brought_to_the_requesters_branch_when_its_check_passes()
    {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        let tasks = answered_request(&mut world, MEMBER_REQUEST);
        let member = task(&tasks, "claude-1");
        // The requester has uncommitted work of its own, which the bringing over must leave as it is.
        std::fs::write(world.worktree(member).join("scratch.txt"), "my notes\n").unwrap();

        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();

        world.request_is("1", "done");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.file(member, "scratch.txt"), "my notes\n");
        let message = world.git_output(member, &["log", "-1", "--format=%B"]);
        assert!(message.contains("Write the walkthrough"), "{message}");
        assert!(message.contains("Brought over from"), "{message}");
        let told = world.told("claude-1", "did step 1.1 of your plan, and its commit");
        assert!(told.contains("Files: docs/guide.md"), "{told}");
        assert_eq!(world.requests().len(), 1, "the result is answered once");
        let answers = world
            .messages_to("claude-1")
            .iter()
            .filter(|text| text.contains("did step 1.1 of your plan"))
            .count();
        assert_eq!(answers, 1);
    }

    #[test]
    fn a_worker_whose_check_fails_has_nothing_brought_over_and_the_step_goes_back_with_the_reason() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
        ]);
        world.script("guide", &commit_script("wrong text", &[]));
        let mut request: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        request["check"] = serde_json::Value::String("grep -q walkthrough docs/guide.md".to_owned());
        let tasks = answered_request(&mut world, &request.to_string());
        let member = task(&tasks, "claude-1");

        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();

        world.request_is("1", "failed");
        let prompt = world.told("claude-1", "is back with you");
        assert!(prompt.contains("is back with you"), "{prompt}");
        assert!(
            prompt.contains("the check `grep -q walkthrough docs/guide.md` failed"),
            "{prompt}"
        );
        assert!(!world.worktree(member).join("docs/guide.md").exists());
        let message = world.git_output(member, &["log", "-1", "--format=%B"]);
        assert!(!message.contains("Brought over from"), "{message}");
    }

    #[test]
    fn a_worker_that_changes_a_file_outside_the_request_has_nothing_brought_over_and_the_file_is_named() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &["docs/notes.md"]));
        let tasks = answered_request(&mut world, MEMBER_REQUEST);
        let member = task(&tasks, "claude-1");

        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();

        world.request_is("1", "failed");
        let prompt = world.told("claude-1", "is back with you");
        assert!(
            prompt.contains("changed files the request did not name: docs/notes.md"),
            "{prompt}"
        );
        assert!(!world.worktree(member).join("docs/guide.md").exists());
    }

    #[test]
    fn a_worker_that_commits_nothing_for_a_request_that_names_files_has_the_step_returned() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,done:Result: done."),
        ]);
        let tasks = answered_request(&mut world, MEMBER_REQUEST);
        let member = task(&tasks, "claude-1");

        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();

        world.request_is("1", "failed");
        let prompt = world.told("claude-1", "is back with you");
        assert!(prompt.contains("the check `exists docs/guide.md` failed"), "{prompt}");
    }

    #[test]
    fn a_commit_that_conflicts_with_the_requesters_branch_is_not_applied_and_the_requester_is_told() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
        ]);
        world.script("guide", &commit_script("the worker version", &[]));
        let tasks = answered_request(&mut world, MEMBER_REQUEST);
        let member = task(&tasks, "claude-1");
        // The requester wrote the same file differently and committed it meanwhile.
        let worktree = world.worktree(member);
        std::fs::create_dir_all(worktree.join("docs")).unwrap();
        std::fs::write(worktree.join("docs/guide.md"), "the requester's text\n").unwrap();
        git(&worktree, &["add", "docs/guide.md"]);
        git(&worktree, &["commit", "-q", "-m", "The requester's own walkthrough"]);
        let head = world.git_output(member, &["rev-parse", "HEAD"]);

        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();

        world.request_is("1", "failed");
        let prompt = world.told("claude-1", "does not apply to your branch as it is now");
        assert!(
            prompt.contains("does not apply to your branch as it is now"),
            "{prompt}"
        );
        assert!(prompt.contains("docs/guide.md"), "{prompt}");
        assert_eq!(world.git_output(member, &["rev-parse", "HEAD"]), head);
        assert_eq!(world.file(member, "docs/guide.md"), "the requester's text\n");
    }
}
