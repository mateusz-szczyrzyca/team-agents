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
            let path = self.worktree(task).join(".fake-runs");
            let deadline = Instant::now() + PATIENCE;
            loop {
                let pending = match std::fs::read_to_string(&path) {
                    Ok(text) => match text.strip_suffix('\n') {
                        Some(complete) => match complete.parse::<u32>() {
                            Ok(runs) => return runs,
                            Err(error) => format!("invalid counter {text:?}: {error}"),
                        },
                        // The stand-in terminates its counter with a newline; without it the
                        // writer may still be between truncating the file and finishing it.
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

        /// Submits delegation feedback for request `id` as `task`.
        fn feedback_as(&self, task: &str, id: i64, verdict: &str) -> String {
            let tool_args = format!(r#"{{"id":"H{id}","verdict":"{verdict}"}}"#);
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
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"delegation_feedback\",\"arguments\":{tool_args}}}}}\n"
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
        // The row is about the prompts of the first tries: a wrong result is not given to the other worker here.
        world.allow_tries(1);
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
            requests.len() == 2 && requests.values().all(|request| request["status"] == "failed")
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

        eventually("the request to be failed", || {
            world.requests()["1"]["status"] == "failed"
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
            // The stand-in's own `.fake-*` files are kept out of git's sight, as the harness keeps its files of a run out:
            // the daemon refuses a result whose worktree holds uncommitted files.
            let exclude = "printf '%s\\n' '.fake-*' >> \"$(git rev-parse --git-path info/exclude)\"\n";
            std::fs::write(dir.join(format!("{name}.sh")), format!("{exclude}{body}")).unwrap();
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
            write!(script, "printf 'x\\n' > {file}\ngit add {file}\n").unwrap();
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

    impl World {
        /// The ID of the team the daemon formed.
        fn team_id(&self) -> String {
            let text = std::fs::read_to_string(self.home.join("state.json")).unwrap();
            let state: serde_json::Value = serde_json::from_str(&text).unwrap();
            state["teams"][0]["id"].as_str().unwrap().to_owned()
        }

        /// Runs `agentbus` with `args` on the bus of the home.
        fn bus_cli(&self, args: &[&str]) {
            let output = Command::new(agentbus_binary())
                .arg(args[0])
                .arg("--dir")
                .arg(self.home.join("bus"))
                .args(&args[1..])
                .env("AGENTS_HOME", &self.home)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "agentbus {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        /// What the daemon would have written when the worker `worker` of `requester` ended request 1 and went down
        /// before the next stage: the worker commits as its script says, the requester's request is on the bus as running,
        /// and the file of the stage `stage` is in the worker's task directory.
        fn worker_ended_and_daemon_down(&self, requester: &str, worker: &str, stage: serde_json::Value, request: &str) {
            let worktree = self.worktree(worker);
            let base = self.git_output(worker, &["rev-parse", "HEAD"]);
            let status = Command::new("bash")
                .arg(self.home.join("fake-scripts/guide.sh"))
                .current_dir(&worktree)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap();
            assert!(status.success());
            let head = self.git_output(worker, &["rev-parse", "HEAD"]);
            std::fs::create_dir_all(self.worktree(requester).join(".agent")).unwrap();
            std::fs::write(self.worktree(requester).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
            let got = answers(&self.delegate_as(requester, request));
            assert!(!got[0].0, "{got:?}");
            self.bus_cli(&["helper-set", "--id", "1", "--status", "running", "--task", worker]);
            let mut stage = stage;
            let head_text = serde_json::Value::String(head);
            if let Some(object) = stage.as_object_mut()
                && object.contains_key("head")
            {
                object.insert("head".to_owned(), head_text);
            }
            let pending = serde_json::json!({
                "request": 1,
                "team": self.team_id(),
                "requester": "claude-1",
                "requester_task": requester,
                "worker": "glm-1",
                "worker_task": worker,
                "summary": "Result: done.",
                "run_seconds": 1,
                "base": base,
                "stage": stage,
            });
            std::fs::write(
                self.home.join("tasks").join(worker).join("worker-request-1.json"),
                serde_json::to_string(&pending).unwrap(),
            )
            .unwrap();
        }

        /// The commits of the worktree of `task` that brought a worker's commit over.
        fn brought_over(&self, task: &str) -> usize {
            self.git_output(task, &["log", "--format=%H", "--grep", "Brought over from"])
                .lines()
                .count()
        }
    }

    /// The request of the member whose check counts its runs, in the worker's worktree.
    fn counting_request() -> String {
        let mut request: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        request["check"] =
            serde_json::Value::String("echo run >> .fake-check-count && test -s docs/guide.md".to_owned());
        request.to_string()
    }

    /// A world with a member and a worker, the daemon up and down again, ready for a request the worker has served.
    fn world_with_a_served_request() -> (World, String, String) {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "sleep"),
            ("glm-1", "worker", "sleep"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);
        let (member, worker) = (task(&tasks, "claude-1").to_owned(), task(&tasks, "glm-1").to_owned());
        for name in ["claude-1", "inv-opus", "glm-1"] {
            world.ended(task(&tasks, name), 1);
        }
        world.sleeps_on_the_bus("glm-1");
        world.stop();
        (world, member, worker)
    }

    #[test]
    fn a_daemon_that_went_down_after_the_workers_end_runs_the_check_once_and_brings_the_commit_over_once() {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(
            &member,
            &worker,
            serde_json::json!({"stage": "ended"}),
            &counting_request(),
        );

        world.start();

        world.request_is("1", "done");
        assert_eq!(world.file(&member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.file(&worker, ".fake-check-count"), "run\n", "the check ran once");
        assert_eq!(world.brought_over(&member), 1);
        world.told("claude-1", "did step 1.1 of your plan");
        let answers = world
            .messages_to("claude-1")
            .iter()
            .filter(|text| text.contains("did step 1.1 of your plan"))
            .count();
        assert_eq!(answers, 1);
    }

    #[test]
    fn a_daemon_that_went_down_after_the_check_does_not_run_it_again() {
        let (mut world, member, worker) = world_with_a_served_request();
        let checked = serde_json::json!({"stage": "checked", "step": "1.1", "files": ["docs/guide.md"], "head": null,
            "committed": true, "outside": [],
            "found": {"outcome": "passed", "line": "echo run >> .fake-check-count && test -s docs/guide.md",
                      "tail": "", "reason": null, "duration_ms": 1, "output": null}});
        world.worker_ended_and_daemon_down(&member, &worker, checked, &counting_request());

        world.start();

        world.request_is("1", "done");
        assert_eq!(world.file(&member, "docs/guide.md"), "the walkthrough\n");
        assert!(
            !world.worktree(&worker).join(".fake-check-count").exists(),
            "the check that was run before the daemon went down is not run again"
        );
        assert_eq!(world.brought_over(&member), 1);
    }

    #[test]
    fn a_daemon_that_went_down_after_the_bringing_over_does_not_bring_the_commit_over_again() {
        let (mut world, member, worker) = world_with_a_served_request();
        let checked = serde_json::json!({"stage": "checked", "step": "1.1", "files": ["docs/guide.md"], "head": null,
            "committed": true, "outside": [],
            "found": {"outcome": "passed", "line": "exists docs/guide.md",
                      "tail": "", "reason": null, "duration_ms": 1, "output": null}});
        world.worker_ended_and_daemon_down(&member, &worker, checked.clone(), MEMBER_REQUEST);
        world.start();
        world.request_is("1", "done");
        assert_eq!(world.brought_over(&member), 1);
        // The daemon is told to go down in the other order: the commit is on the branch, but the file of the request still
        // names the stage before the bringing over.
        world.stop();
        let pending = serde_json::json!({
            "request": 1, "team": world.team_id(), "requester": "claude-1", "requester_task": member,
            "worker": "glm-1", "worker_task": worker, "summary": "Result: done.", "run_seconds": 1,
            "stage": {"stage": "checked", "step": "1.1", "files": ["docs/guide.md"],
                      "head": world.git_output(&worker, &["rev-parse", "HEAD"]), "committed": true, "outside": [],
                      "found": {"outcome": "passed", "line": "exists docs/guide.md", "tail": "", "reason": null,
                                "duration_ms": 1, "output": null}},
        });
        std::fs::write(
            world.home.join("tasks").join(&worker).join("worker-request-1.json"),
            serde_json::to_string(&pending).unwrap(),
        )
        .unwrap();

        world.start();

        eventually("the file of the request to be settled", || {
            !world
                .home
                .join("tasks")
                .join(&worker)
                .join("worker-request-1.json")
                .exists()
        });
        assert_eq!(world.brought_over(&member), 1, "the commit is on the branch once");
    }

    /// The request of the member for the step 1.1 with the file `docs/other.md`.
    const OTHER_REQUEST: &str = r#"{"goal":"another file","workdir":"/tmp","steps":"write the file","returns":"the path","for":"1.1","files":"docs/other.md","check":"exists docs/other.md","do":"Write the other file."}"#;

    #[test]
    fn a_second_request_of_a_worker_does_not_bring_over_the_rejected_result_of_its_first() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide,script:other"),
        ]);
        world.script("guide", &commit_script("wrong text", &[]));
        world.script(
            "other",
            "mkdir -p docs\nprintf 'other\\n' > docs/other.md\ngit add docs/other.md\ngit -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the other file'\n",
        );
        let mut first: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        first["check"] = serde_json::Value::String("grep -q walkthrough docs/guide.md".to_owned());
        let tasks = answered_request(&mut world, &first.to_string());
        let member = task(&tasks, "claude-1");
        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();
        world.request_is("1", "failed");
        world.sleeps_on_the_bus("glm-1");

        let got = answers(&world.delegate_as(member, OTHER_REQUEST));
        assert!(!got[0].0, "{got:?}");

        world.request_is("2", "done");
        assert_eq!(world.file(member, "docs/other.md"), "other\n");
        assert!(
            !world.worktree(member).join("docs/guide.md").exists(),
            "the rejected result of the first request is not brought over with the second"
        );
    }

    #[test]
    fn a_check_that_takes_long_does_not_stop_the_daemon_from_answering() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        let mut request: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        request["check"] = serde_json::Value::String(HELD_CHECK.to_owned());
        let tasks = answered_request(&mut world, &request.to_string());
        let (member, worker) = (task(&tasks, "claude-1"), task(&tasks, "glm-1"));
        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();
        let started = world.worktree(worker).join(".fake-check-started");
        eventually("the check to start", || started.exists());

        // The check holds until the test lets it go: a daemon that waited for it would answer after it gave up, and the
        // request would have failed.
        let output = world.agent(&["status"]);

        assert!(output.status.success());
        std::fs::write(world.worktree(worker).join(".fake-check-go"), "").unwrap();
        world.request_is("1", "done");
    }

    /// A check that says it started and holds until the test says it may end (it gives up after thirty seconds), in the
    /// worker's worktree.
    const HELD_CHECK: &str = "touch .fake-check-started && for i in $(seq 1 300); do test -e .fake-check-go && break; sleep 0.1; done && test -e .fake-check-go && test -s docs/guide.md";

    /// A world with a member and two workers, the first of which commits the guide, and the request `request` of the member,
    /// served by the first worker; returns the tasks by name.
    fn two_workers_one_request(world: &mut World, request: &str) -> Vec<(String, String)> {
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("member", Some(MEMBER_PLAN), &[request]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1", "glm-2"]);
        world.ended(task(&tasks, "glm-1"), 1);
        world.sleeps_on_the_bus("glm-1");
        world.sleeps_on_the_bus("glm-2");
        tasks
    }

    #[test]
    fn a_worker_whose_result_is_being_checked_is_not_given_the_next_request() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
            ("glm-2", "worker", "sleep,sleep"),
        ]);
        let mut request: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        request["check"] = serde_json::Value::String(HELD_CHECK.to_owned());
        let tasks = two_workers_one_request(&mut world, &request.to_string());
        let (member, first) = (task(&tasks, "claude-1"), task(&tasks, "glm-1"));
        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();
        let started = world.worktree(first).join(".fake-check-started");
        eventually("the check of the first request to start", || started.exists());

        let second = answers(&world.delegate_as(member, MEMBER_REQUEST));

        assert!(!second[0].0, "{second:?}");
        eventually("the second request to be given to a worker", || {
            world
                .requests()
                .get("2")
                .is_some_and(|request| !request["worker"].is_null())
        });
        assert_eq!(
            world.requests()["2"]["worker"],
            "glm-2",
            "the worker whose result is being checked was given the next request"
        );
        std::fs::write(world.worktree(first).join(".fake-check-go"), "").unwrap();
        world.request_is("1", "done");
    }

    #[test]
    fn a_worker_that_leaves_a_requested_file_uncommitted_has_the_step_returned() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
            ("glm-1", "worker", "sleep,script:guide"),
        ]);
        world.script(
            "guide",
            &format!(
                "{}printf 'three\\n' > docs/count.txt\n",
                commit_script("the walkthrough", &[])
            ),
        );
        let mut request: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        request["files"] = serde_json::Value::String("docs/guide.md, docs/count.txt".to_owned());
        request["check"] = serde_json::Value::String("exists docs/count.txt".to_owned());
        let tasks = answered_request(&mut world, &request.to_string());
        let member = task(&tasks, "claude-1");

        std::fs::write(world.worktree(member).join(".fake-ask"), "").unwrap();

        world.request_is("1", "failed");
        let told = world.told("claude-1", "is back with you");
        assert!(told.contains("uncommitted"), "{told}");
        assert!(told.contains("docs/count.txt"), "{told}");
        assert!(!world.worktree(member).join("docs/guide.md").exists());
    }

    /// The file of the request `1`, as the test wrote it.
    fn pending_file(world: &World, worker: &str) -> PathBuf {
        world.home.join("tasks").join(worker).join("worker-request-1.json")
    }

    #[test]
    fn a_result_whose_base_or_files_cannot_be_read_is_returned_and_not_brought_over() {
        for base in [None, Some("0000000000000000000000000000000000000001")] {
            let (mut world, member, worker) = world_with_a_served_request();
            world.worker_ended_and_daemon_down(&member, &worker, serde_json::json!({"stage": "ended"}), MEMBER_REQUEST);
            let path = pending_file(&world, &worker);
            let mut pending: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            match base {
                None => drop(pending.as_object_mut().unwrap().remove("base")),
                Some(base) => pending["base"] = serde_json::Value::String(base.to_owned()),
            }
            std::fs::write(&path, pending.to_string()).unwrap();

            world.start();

            world.request_is("1", "failed");
            world.told("claude-1", "is back with you");
            assert!(!world.worktree(&member).join("docs/guide.md").exists(), "base {base:?}");
            assert_eq!(world.brought_over(&member), 0, "base {base:?}");
        }
    }

    /// The stage file of a result settled as returned, as the daemon writes it before it tells the requester.
    fn settled_returned() -> serde_json::Value {
        serde_json::json!({"stage": "settled",
            "checked": {"step": "1.1", "files": ["docs/guide.md"], "head": null, "committed": true, "outside": [],
                        "found": null},
            "settlement": {"settled": "returned", "reasons": ["the check failed"]}})
    }

    #[test]
    fn a_daemon_that_went_down_after_the_settling_tells_the_requester_once_and_removes_the_file() {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_returned(), MEMBER_REQUEST);

        world.start();

        world.request_is("1", "failed");
        eventually("the file of the request to be removed", || {
            !pending_file(&world, &worker).exists()
        });
        world.told("claude-1", "is back with you");
        world.stop();
        world.start();
        world.after_a_look(&member, &worker);
        assert_eq!(
            world.count_told("claude-1", "Your request H1 to the workers is back with you"),
            1
        );
    }

    #[test]
    fn a_damaged_file_of_a_request_does_not_stop_the_daemon_nor_the_requests_beside_it() {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_returned(), MEMBER_REQUEST);
        std::fs::write(
            world.home.join("tasks").join(&worker).join("worker-request-7.json"),
            "not json",
        )
        .unwrap();

        world.start();

        world.request_is("1", "failed");
        let output = world.agent(&["status"]);
        assert!(output.status.success(), "the daemon still answers");
    }

    /// How the file of request 1 is damaged by the test.
    #[derive(Clone, Copy, Debug)]
    enum Damage {
        /// Cut in the middle of its text.
        Truncated,
        /// Complete, with a field this version does not know.
        UnknownField,
        /// Cannot be read at all: a directory stands where the file is.
        Unreadable,
    }

    /// Damages the file of request 1 as `damage` says; returns what the file held before.
    fn damage_the_file(path: &Path, damage: Damage) -> String {
        let text = std::fs::read_to_string(path).unwrap();
        match damage {
            Damage::Truncated => std::fs::write(path, &text.as_bytes()[..text.len() / 2]).unwrap(),
            Damage::UnknownField => {
                let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
                value["surprise"] = serde_json::json!(1);
                std::fs::write(path, value.to_string()).unwrap();
            }
            Damage::Unreadable => {
                std::fs::remove_file(path).unwrap();
                std::fs::create_dir(path).unwrap();
            }
        }
        text
    }

    /// The entries of the task directory of `worker` that keep the damaged file of request 1 aside.
    fn kept_aside(world: &World, worker: &str) -> Vec<PathBuf> {
        let dir = world.home.join("tasks").join(worker);
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("worker-request-1.json") && name != "worker-request-1.json")
            })
            .collect()
    }

    /// Request 1 of a worker has a damaged file, request 2 of the same team a good one: both are answered as wrong results
    /// and told to the requester, the damaged file is kept aside under another name and logged, and a restart of the daemon
    /// does not meet it again.
    fn a_damaged_file_is_settled_as_a_wrong_result_and_set_aside(damage: Damage) {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_returned(), MEMBER_REQUEST);
        let task_dir = world.home.join("tasks").join(&worker);
        let second = answers(&world.delegate_as(&member, MEMBER_REQUEST));
        assert!(!second[0].0, "{second:?}");
        world.bus_cli(&["helper-set", "--id", "2", "--status", "running", "--task", &worker]);
        let mut pending: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(task_dir.join("worker-request-1.json")).unwrap()).unwrap();
        pending["request"] = serde_json::json!(2);
        std::fs::write(task_dir.join("worker-request-2.json"), pending.to_string()).unwrap();
        let before = damage_the_file(&pending_file(&world, &worker), damage);

        world.start();

        world.request_is("1", "failed");
        world.request_is("2", "failed");
        let first = world.told("claude-1", "Your request H1 to the workers is back with you");
        assert!(
            first.contains("worker-request-1.json"),
            "the requester is told which file: {first}"
        );
        world.told("claude-1", "Your request H2 to the workers is back with you");
        assert!(
            output_of(&world.agent(&["status"])).contains("claude-1"),
            "the daemon still answers"
        );
        assert!(
            !pending_file(&world, &worker).exists(),
            "the damaged file is not met again"
        );
        let aside = kept_aside(&world, &worker);
        assert_eq!(aside.len(), 1, "{aside:?}");
        if !matches!(damage, Damage::UnknownField | Damage::Truncated) {
            assert!(aside[0].is_dir(), "kept as it was");
        } else if matches!(damage, Damage::Truncated) {
            assert_eq!(
                std::fs::read(&aside[0]).unwrap(),
                &before.as_bytes()[..before.len() / 2]
            );
        }
        eventually("the damaged file to be logged with its name", || {
            output_of(&world.agent(&["log", "--json", "--event", "worker_request_damaged"]))
                .contains("worker-request-1.json")
        });

        world.stop();
        world.start();
        world.after_a_look(&member, &worker);
        let told = world.count_told("claude-1", "Your request H1 to the workers is back with you");
        assert_eq!(told, 1, "a restart does not settle the damaged file again");
        assert_eq!(kept_aside(&world, &worker).len(), 1);
    }

    /// What a command printed on its standard output.
    fn output_of(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn a_truncated_file_of_a_request_is_settled_as_a_wrong_result_and_set_aside() {
        a_damaged_file_is_settled_as_a_wrong_result_and_set_aside(Damage::Truncated);
    }

    #[test]
    fn a_file_of_a_request_with_an_unknown_field_is_settled_as_a_wrong_result_and_set_aside() {
        a_damaged_file_is_settled_as_a_wrong_result_and_set_aside(Damage::UnknownField);
    }

    #[test]
    fn an_unreadable_file_of_a_request_is_settled_as_a_wrong_result_and_set_aside() {
        a_damaged_file_is_settled_as_a_wrong_result_and_set_aside(Damage::Unreadable);
    }

    /// Gives a directory its permissions back when the test ends, so that the scratch home can be removed.
    struct Writable(PathBuf);

    impl Drop for Writable {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            // A test that cannot give them back leaves a scratch directory behind; there is nothing to do about that here.
            if let Err(error) = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755)) {
                eprintln!("cannot restore the permissions of {:?}: {error}", self.0);
            }
        }
    }

    #[test]
    fn a_request_whose_stage_cannot_be_written_is_settled_as_a_wrong_result_and_the_daemon_goes_on() {
        use std::os::unix::fs::PermissionsExt;
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, serde_json::json!({"stage": "ended"}), MEMBER_REQUEST);
        let task_dir = world.home.join("tasks").join(&worker);
        std::fs::set_permissions(&task_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let _writable = Writable(task_dir);

        world.start();

        world.request_is("1", "failed");
        let told = world.told("claude-1", "Your request H1 to the workers is back with you");
        assert!(told.contains("cannot keep"), "{told}");
        assert!(
            output_of(&world.agent(&["status"])).contains("claude-1"),
            "the daemon still answers"
        );
    }

    /// Like [`world_with_a_served_request`], with a second worker `glm-2` whose task directory the test may write to.
    fn world_with_two_workers_and_a_served_request() -> (World, String, String, String) {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "sleep"),
            ("glm-1", "worker", "sleep"),
            ("glm-2", "worker", "sleep"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1", "glm-2"]);
        let (member, first, second) = (
            task(&tasks, "claude-1").to_owned(),
            task(&tasks, "glm-1").to_owned(),
            task(&tasks, "glm-2").to_owned(),
        );
        for name in ["claude-1", "inv-opus", "glm-1", "glm-2"] {
            world.ended(task(&tasks, name), 1);
        }
        world.sleeps_on_the_bus("glm-1");
        world.sleeps_on_the_bus("glm-2");
        world.stop();
        (world, member, first, second)
    }

    impl World {
        /// Lets a request be given to `tries` workers in all (`[workers] tries`): `1` never gives it to a second worker.
        fn allow_tries(&self, tries: u32) {
            let mut config = std::fs::read_to_string(self.home.join("agents.toml")).unwrap();
            write!(config, "[workers]\ntries = {tries}\n").unwrap();
            std::fs::write(self.home.join("agents.toml"), config).unwrap();
        }

        /// The file of a request `number` of `member` that `worker` has ended and the daemon has settled as returned.
        fn settled_file(&self, number: u64, member: &str, worker: &str) -> serde_json::Value {
            serde_json::json!({
                "request": number, "team": self.team_id(), "requester": "claude-1", "requester_task": member,
                "worker": "glm-2", "worker_task": worker, "summary": "Result: done.", "run_seconds": 1,
                "stage": settled_returned(),
            })
        }

        /// How many messages to `name` hold `needle`.
        fn count_told(&self, name: &str, needle: &str) -> usize {
            self.messages_to(name)
                .iter()
                .filter(|text| text.contains(needle))
                .count()
        }

        /// Puts one more request of `member` on the bus as running with `worker` (a worker whose task directory the test
        /// may write to) and returns its number; the bus does not take a request while a result of the member is unrated.
        fn one_more_request(&self, member: &str, worker: &str) -> u64 {
            let number = u64::try_from(self.requests().len()).unwrap() + 1;
            let answer = answers(&self.delegate_as(member, MEMBER_REQUEST));
            assert!(!answer[0].0, "{answer:?}");
            self.bus_cli(&[
                "helper-set",
                "--id",
                &number.to_string(),
                "--status",
                "running",
                "--task",
                worker,
            ]);
            number
        }

        /// Gives the daemon the request `number` of `member` to settle, as ended by `worker`, and waits until the requester
        /// is told: the daemon has looked at the team once more after everything the test did before, without a pause to
        /// guess the time of.
        fn settle_as_a_look(&self, number: u64, member: &str, worker: &str) {
            std::fs::write(
                self.home
                    .join("tasks")
                    .join(worker)
                    .join(format!("worker-request-{number}.json")),
                self.settled_file(number, member, worker).to_string(),
            )
            .unwrap();
            self.told(
                "claude-1",
                &format!("Your request H{number} to the workers is back with you"),
            );
        }

        /// [`World::one_more_request`] and [`World::settle_as_a_look`] in one.
        fn after_a_look(&self, member: &str, worker: &str) {
            let number = self.one_more_request(member, worker);
            self.settle_as_a_look(number, member, worker);
        }
    }

    #[test]
    fn a_request_whose_stage_cannot_be_written_is_answered_once_not_at_every_look() {
        use std::os::unix::fs::PermissionsExt;
        let (mut world, member, worker, other) = world_with_two_workers_and_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, serde_json::json!({"stage": "ended"}), MEMBER_REQUEST);
        let task_dir = world.home.join("tasks").join(&worker);
        let path = pending_file(&world, &worker);
        let mut pending: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        pending.as_object_mut().unwrap().remove("base");
        std::fs::write(&path, pending.to_string()).unwrap();
        std::fs::set_permissions(&task_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let _writable = Writable(task_dir);

        world.start();
        world.told("claude-1", "Your request H1 to the workers is back with you");
        world.after_a_look(&member, &other);

        assert_eq!(
            world.count_told("claude-1", "Your request H1 to the workers is back with you"),
            1
        );
        assert_eq!(world.requests()["1"]["status"], "failed");
    }

    #[test]
    fn a_damaged_file_of_a_request_the_bus_has_answered_does_not_answer_it_again() {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_returned(), MEMBER_REQUEST);
        let sentinel = world.one_more_request(&member, &worker);
        world.bus_cli(&[
            "helper-set",
            "--id",
            "1",
            "--status",
            "done",
            "--summary",
            "Done by the worker.",
        ]);
        damage_the_file(&pending_file(&world, &worker), Damage::Truncated);

        world.start();
        world.settle_as_a_look(sentinel, &member, &worker);

        assert_eq!(
            world.requests()["1"]["status"],
            "done",
            "the answer of the bus was not overwritten"
        );
        assert_eq!(
            world.count_told("claude-1", "Your request H1 to the workers is back with you"),
            0
        );
        assert!(!pending_file(&world, &worker).exists());
        assert_eq!(kept_aside(&world, &worker).len(), 1);
    }

    #[test]
    fn a_file_whose_request_is_not_the_one_in_its_name_is_damage_and_is_not_settled_again_and_again() {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_returned(), MEMBER_REQUEST);
        let task_dir = world.home.join("tasks").join(&worker);
        std::fs::rename(
            task_dir.join("worker-request-1.json"),
            task_dir.join("worker-request-5.json"),
        )
        .unwrap();

        world.start();
        world.told("claude-1", "Your request H1 to the workers is back with you");
        world.after_a_look(&member, &worker);

        assert!(
            !task_dir.join("worker-request-5.json").exists(),
            "the file is set aside"
        );
        assert!(task_dir.join("worker-request-5.json.damaged").exists(), "and kept");
        // The request the file holds is answered too, once: the bus had it running for this worker.
        assert_eq!(
            world.count_told("claude-1", "Your request H1 to the workers is back with you"),
            1
        );
        assert_eq!(world.requests()["1"]["status"], "failed");
    }

    /// The member's request on a team of `workers` (each with its own script), served by the first of them, whose result is
    /// wrong or right as its script says; the world is started and the first request answered by the bus as given.
    /// Returns the tasks by name.
    fn second_try_world(
        workers: &[(&'static str, &'static str)],
        tries: Option<u32>,
    ) -> (World, Vec<(String, String)>) {
        let mut specs: Vec<AgentSpec> = vec![
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker-after:member,sleep"),
        ];
        specs.extend(workers.iter().map(|(name, scenario)| (*name, "worker", *scenario)));
        let mut world = World::new(&specs);
        if let Some(tries) = tries {
            let mut config = std::fs::read_to_string(world.home.join("agents.toml")).unwrap();
            write!(config, "[workers]\ntries = {tries}\n").unwrap();
            std::fs::write(world.home.join("agents.toml"), config).unwrap();
        }
        world.script("nothing", "true\n");
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();
        let mut names = vec!["claude-1", "inv-opus"];
        names.extend(workers.iter().map(|(name, _)| *name));
        let tasks = world.run(&names);
        world.ended(task(&tasks, workers[0].0), 1);
        for (name, _) in workers {
            world.sleeps_on_the_bus(name);
        }
        std::fs::write(world.worktree(task(&tasks, "claude-1")).join(".fake-ask"), "").unwrap();
        (world, tasks)
    }

    #[test]
    fn a_wrong_result_is_given_once_to_another_worker_with_the_reason_and_the_commit_of_the_second_is_brought_over() {
        let (world, tasks) = second_try_world(
            &[("glm-1", "sleep,script:nothing"), ("glm-2", "sleep,script:guide")],
            None,
        );
        let (member, second) = (task(&tasks, "claude-1"), task(&tasks, "glm-2"));

        world.request_is("1", "done");

        assert_eq!(world.requests()["1"]["worker"], "glm-2");
        let again = world.told("claude-1", "went once more to the worker glm-2");
        assert!(again.contains("the check `exists docs/guide.md` failed"), "{again}");
        let prompt = world.file(second, ".fake-prompt-2");
        assert!(prompt.contains("A REQUEST FROM claude-1"), "{prompt}");
        assert!(prompt.contains("served this request before you"), "{prompt}");
        assert!(prompt.contains("glm-1"), "{prompt}");
        assert!(prompt.contains("the check `exists docs/guide.md` failed"), "{prompt}");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.brought_over(member), 1);
        assert_eq!(
            world.count_told("claude-1", "went once more to the worker glm-2"),
            1,
            "the requester is told once"
        );
    }

    #[test]
    fn two_wrong_results_return_the_step_once_with_both_reasons_and_no_third_worker_starts() {
        let (world, tasks) = second_try_world(
            &[
                ("glm-1", "sleep,script:nothing"),
                ("glm-2", "sleep,script:nothing"),
                ("glm-3", "sleep,script:guide"),
            ],
            None,
        );

        world.request_is("1", "failed");

        let told = world.told("claude-1", "Your request H1 to the workers is back with you");
        assert!(told.contains("the first result: "), "{told}");
        assert!(told.contains("the second result: "), "{told}");
        assert_eq!(world.runs(task(&tasks, "glm-3")), 1, "a third worker did not start");
        assert_eq!(world.runs(task(&tasks, "glm-2")), 2);
        assert_eq!(
            world.count_told("claude-1", "Your request H1 to the workers is back with you"),
            1
        );
    }

    #[test]
    fn a_request_that_allows_one_try_is_never_given_to_a_second_worker() {
        let (world, tasks) = second_try_world(
            &[("glm-1", "sleep,script:nothing"), ("glm-2", "sleep,script:guide")],
            Some(1),
        );

        world.request_is("1", "failed");

        let told = world.told("claude-1", "Your request H1 to the workers is back with you");
        assert!(!told.contains("the second result"), "{told}");
        assert_eq!(
            world.runs(task(&tasks, "glm-2")),
            1,
            "the second worker was not given it"
        );
        assert_eq!(world.count_told("claude-1", "went once more"), 0);
    }

    /// The stage file of a result that is wrong and goes to another worker, as the daemon writes it before it asks.
    fn settled_again() -> serde_json::Value {
        serde_json::json!({"stage": "settled",
            "checked": {"step": "1.1", "files": ["docs/guide.md"], "head": null, "committed": true, "outside": [],
                        "found": null},
            "settlement": {"settled": "again", "reasons": ["the check failed"]}})
    }

    #[test]
    fn a_daemon_that_went_down_before_the_second_try_gives_the_request_to_another_worker_once() {
        let (mut world, member, worker, other) = world_with_two_workers_and_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_again(), MEMBER_REQUEST);

        world.start();

        eventually("the request to be given to the second worker", || {
            world
                .requests()
                .get("1")
                .is_some_and(|request| request["worker"] == "glm-2")
        });
        world.told("claude-1", "went once more to the worker glm-2");
        eventually("the file of the first try to be removed", || {
            !pending_file(&world, &worker).exists()
        });
        world.after_a_look(&member, &other);
        assert_eq!(world.count_told("claude-1", "went once more to the worker glm-2"), 1);
        assert_eq!(world.runs(&other), 2, "the second worker was given the request once");
    }

    #[test]
    fn a_request_the_second_worker_holds_is_not_given_again_by_the_file_of_the_first_whose_task_the_bus_still_names() {
        let (mut world, member, worker, other) = world_with_two_workers_and_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_again(), MEMBER_REQUEST);
        // The second worker took the request, but the update that records its task has not reached the bus: the bus names
        // glm-2 and still has the task of the first worker.
        world.bus_cli(&["helper-set", "--id", "1", "--status", "queued"]);
        world.bus_cli(&["helper-next", "--worker", "glm-2"]);
        assert_eq!(world.requests()["1"]["helper_task"], worker.as_str());
        assert_eq!(world.requests()["1"]["worker"], "glm-2");
        let sentinel = world.one_more_request(&member, &other);

        world.start();
        world.settle_as_a_look(sentinel, &member, &other);

        let request = &world.requests()["1"];
        assert_eq!(
            request["status"], "starting",
            "the request was not queued again: {request}"
        );
        assert_eq!(request["worker"], "glm-2");
        assert!(
            !pending_file(&world, &worker).exists(),
            "the file of the first result is dropped"
        );
        assert_eq!(world.count_told("claude-1", "went once more"), 0);
        assert_eq!(world.runs(&other), 1, "no worker was started for it");
    }

    #[test]
    fn a_second_worker_that_fails_leaves_no_try_file_and_the_requester_is_told() {
        let (world, tasks) = second_try_world(&[("glm-1", "sleep,script:nothing"), ("glm-2", "sleep,fail")], None);

        world.request_is("1", "failed");

        world.told("claude-1", "Your request H1 to the worker glm-2 failed");
        let dir = world.home.join("tasks").join(task(&tasks, "glm-2"));
        let tries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("worker-try-"))
            .collect();
        assert!(tries.is_empty(), "the file of the try stays: {tries:?}");
    }

    #[test]
    fn a_file_of_a_request_the_bus_does_not_know_is_not_taken_as_running_at_its_checked_stage() {
        let (mut world, member, worker) = world_with_a_served_request();
        world.worker_ended_and_daemon_down(&member, &worker, settled_returned(), MEMBER_REQUEST);
        let sentinel = world.one_more_request(&member, &worker);
        let mut pending: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(pending_file(&world, &worker)).unwrap()).unwrap();
        pending["request"] = serde_json::json!(99);
        pending["stage"] = serde_json::json!({"stage": "checked", "step": "1.1", "files": ["docs/guide.md"],
            "head": world.git_output(&worker, &["rev-parse", "HEAD"]), "committed": true, "outside": [],
            "found": {"outcome": "passed", "line": "exists docs/guide.md", "tail": "", "reason": null,
                      "duration_ms": 1, "output": null}});
        let path = world.home.join("tasks").join(&worker).join("worker-request-99.json");
        std::fs::write(&path, pending.to_string()).unwrap();

        world.start();
        world.settle_as_a_look(sentinel, &member, &worker);
        eventually("request 99's pending file to be removed", || !path.exists());

        assert!(!path.exists(), "the file of the unknown request is dropped");
        assert_eq!(
            world.brought_over(&member),
            0,
            "nothing was brought over for a request the bus forgot"
        );
    }

    #[test]
    fn a_request_the_bus_no_longer_has_has_its_file_dropped() {
        let (mut world, _member, worker) = world_with_a_served_request();
        let pending = serde_json::json!({
            "request": 99, "team": world.team_id(), "requester": "claude-1",
            "requester_task": "task-20261122-120000-aaaa", "worker": "glm-1", "worker_task": worker,
            "summary": "Result: done.", "run_seconds": 1, "stage": {"stage": "ended"},
        });
        let path = world.home.join("tasks").join(&worker).join("worker-request-99.json");
        std::fs::write(&path, pending.to_string()).unwrap();

        world.start();

        eventually("the file of the unknown request to be dropped", || !path.exists());
    }

    /// The workspace of a request of `member` served by a worker task that is not a task of the crew: its worktree on
    /// the branch `agents/<worker>/<worker_task>` at the requester's committed end, the old-format OPEN record in the
    /// member's task directory, the pending (without a base) in the worker task's directory, and the request running
    /// on the bus for that worker task. When `guide` the guide is committed in the workspace, as the worker's run
    /// would have left it. Returns the branch.
    fn workspace_of(world: &World, member: &str, worker: &str, worker_task: &str, guide: bool) -> String {
        let base = world.git_output(member, &["rev-parse", "HEAD"]);
        let branch = format!("agents/{worker}/{worker_task}");
        let path = world.worktree(worker_task);
        world.git_output(
            member,
            &["worktree", "add", "-b", &branch, path.to_str().unwrap(), &base],
        );
        if guide {
            let status = Command::new("bash")
                .arg(world.home.join("fake-scripts/guide.sh"))
                .current_dir(&path)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap();
            assert!(status.success());
        }
        let record = serde_json::json!({
            "request": 1,
            "requester": "claude-1",
            "requester_task": member,
            "requester_round": "round-20261124-090000-aaaa",
            "requester_repo": "example",
            "worker": worker,
            "worker_task": worker_task,
            "branch": branch,
            "base": base,
            "stage": {"workspace_stage": "working"},
        });
        std::fs::write(
            world.home.join("tasks").join(member).join("request-workspace-1.json"),
            record.to_string(),
        )
        .unwrap();
        let pending = serde_json::json!({
            "request": 1,
            "team": world.team_id(),
            "requester": "claude-1",
            "requester_task": member,
            "worker": worker,
            "worker_task": worker_task,
            "summary": "Result: done.",
            "run_seconds": 1,
            "stage": {"stage": "ended"},
        });
        let task_dir = world.home.join("tasks").join(worker_task);
        std::fs::create_dir_all(&task_dir).unwrap();
        std::fs::write(task_dir.join("worker-request-1.json"), pending.to_string()).unwrap();
        std::fs::write(world.worktree(member).join("scratch.txt"), "my notes\n").unwrap();
        std::fs::create_dir_all(world.worktree(member).join(".agent")).unwrap();
        std::fs::write(world.worktree(member).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        let number = world.one_more_request(member, worker_task);
        assert_eq!(number, 1, "the request is the first of the bus");
        branch
    }

    #[test]
    fn a_pending_and_try_of_the_main_era_settle_a_served_request() {
        let (mut world, member, worker) = world_with_a_served_request();
        // The worker's run committed the guide in its worktree.
        let base = world.git_output(&worker, &["rev-parse", "HEAD"]);
        let status = Command::new("bash")
            .arg(world.home.join("fake-scripts/guide.sh"))
            .current_dir(world.worktree(&worker))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::create_dir_all(world.worktree(&member).join(".agent")).unwrap();
        std::fs::write(world.worktree(&member).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        let got = answers(&world.delegate_as(&member, &counting_request()));
        assert!(!got[0].0, "{got:?}");
        world.bus_cli(&["helper-set", "--id", "1", "--status", "running", "--task", &worker]);
        // The files the daemon of the main era would have written: the pending of the ended run and its try, with the
        // ids of this world patched in.
        let mut pending: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(script("worker-request-1.json")).unwrap()).unwrap();
        pending["team"] = serde_json::json!(world.team_id());
        pending["requester_task"] = serde_json::json!(member);
        pending["worker_task"] = serde_json::json!(worker);
        pending["base"] = serde_json::json!(base);
        std::fs::write(
            world.home.join("tasks").join(&worker).join("worker-request-1.json"),
            pending.to_string(),
        )
        .unwrap();
        std::fs::write(
            world.home.join("tasks").join(&worker).join("worker-try-1.json"),
            std::fs::read_to_string(script("worker-try-1.json")).unwrap(),
        )
        .unwrap();

        world.start();

        world.request_is("1", "done");
        assert_eq!(world.file(&member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.file(&worker, ".fake-check-count"), "run\n", "the check ran once");
        assert_eq!(world.brought_over(&member), 1);
        world.told("claude-1", "did step 1.1 of your plan");
        assert_eq!(world.count_told("claude-1", "did step 1.1 of your plan"), 1);
        eventually("the files of the request to be removed", || {
            !world
                .home
                .join("tasks")
                .join(&worker)
                .join("worker-request-1.json")
                .exists()
                && !world
                    .home
                    .join("tasks")
                    .join(&worker)
                    .join("worker-try-1.json")
                    .exists()
        });
    }

    #[test]
    fn an_old_open_record_with_its_pending_is_settled_in_the_workspace_and_the_workspace_is_released() {
        let (mut world, member, _worker) = world_with_a_served_request();
        let worker_task = "task-20261124-090000-cccc";
        let branch = workspace_of(&world, &member, "glm-1", worker_task, true);
        assert!(
            !world.worktree(&member).join("docs/guide.md").exists(),
            "the guide is only in the workspace, not yet in the requester's worktree"
        );

        world.start();

        world.request_is("1", "done");
        assert_eq!(world.file(&member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.brought_over(&member), 1);
        assert_eq!(
            world.file(&member, "scratch.txt"),
            "my notes\n",
            "the requester's own file is untouched"
        );
        let record = world.home.join("tasks").join(&member).join("request-workspace-1.json");
        let worktree = world.home.join("worktrees/example").join(worker_task);
        let pending = world.home.join("tasks").join(worker_task).join("worker-request-1.json");
        eventually("the workspace to be released", || {
            !record.exists()
                && !worktree.exists()
                && !pending.exists()
                && world.git_output(&member, &["branch", "--list", &branch]).is_empty()
        });
    }

    #[test]
    fn a_workspace_whose_pending_is_malformed_is_quarantined_and_never_released_across_passes_and_reload() {
        let (mut world, member, worker) = world_with_a_served_request();
        let worker_task = "task-20261124-090000-cccc";
        let branch = workspace_of(&world, &member, "glm-1", worker_task, true);
        // The record is at Ended and its pending is not JSON: the pending cannot be used, so it is set aside and kept,
        // and the workspace is never released on its behalf.
        let task_dir = world.home.join("tasks").join(worker_task);
        std::fs::write(task_dir.join("worker-request-1.json"), "not json at all").unwrap();
        let record_path = world.home.join("tasks").join(&member).join("request-workspace-1.json");
        let mut record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
        record["stage"] = serde_json::json!({"workspace_stage": "ended"});
        std::fs::write(&record_path, record.to_string()).unwrap();
        let worktree = world.home.join("worktrees/example").join(worker_task);

        world.start();

        // The first settlement pass sets the malformed pending aside, keeping it under a `.damaged` name.
        eventually("the malformed pending to be set aside", || {
            task_dir.join("worker-request-1.json.damaged").exists()
        });
        // A later pass must not release the quarantined workspace: its record, worktree and branch all stay.
        let number = world.one_more_request(&member, &worker);
        world.settle_as_a_look(number, &member, &worker);
        assert!(record_path.exists(), "the record is kept after a later pass");
        assert!(worktree.exists(), "the worktree is kept after a later pass");
        assert!(
            world
                .git_output(&member, &["branch", "--list", &branch])
                .contains(&branch),
            "the branch is kept after a later pass"
        );
        assert!(
            task_dir.join("worker-request-1.json.damaged").exists(),
            "the malformed pending is kept"
        );
        // A daemon that restarts still keeps the quarantined workspace and its malformed pending.
        world.stop();
        world.start();
        let number = world.one_more_request(&member, &worker);
        world.settle_as_a_look(number, &member, &worker);
        assert!(record_path.exists(), "the record is kept after a reload");
        assert!(worktree.exists(), "the worktree is kept after a reload");
        assert!(
            task_dir.join("worker-request-1.json.damaged").exists(),
            "the malformed pending is kept after a reload"
        );
    }

    #[test]
    fn a_workspace_whose_pending_mismatches_is_quarantined_and_never_released_across_passes_and_reload() {
        let (mut world, member, worker) = world_with_a_served_request();
        let worker_task = "task-20261124-090000-cccc";
        let branch = workspace_of(&world, &member, "glm-1", worker_task, true);
        // The pending is a valid request for a different worker task than the record: it mismatches the record, so it
        // is set aside and kept, and the workspace is never released on its behalf.
        let mismatching = serde_json::json!({
            "request": 1,
            "team": world.team_id(),
            "requester": "claude-1",
            "requester_task": member,
            "worker": "glm-1",
            "worker_task": "task-20261124-090000-0000",
            "summary": "Result: done.",
            "run_seconds": 1,
            "stage": {"stage": "ended"},
        });
        let task_dir = world.home.join("tasks").join(worker_task);
        std::fs::write(task_dir.join("worker-request-1.json"), mismatching.to_string()).unwrap();
        let record_path = world.home.join("tasks").join(&member).join("request-workspace-1.json");
        let mut record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
        record["stage"] = serde_json::json!({"workspace_stage": "ended"});
        std::fs::write(&record_path, record.to_string()).unwrap();
        let worktree = world.home.join("worktrees/example").join(worker_task);

        world.start();

        eventually("the mismatching pending to be set aside", || {
            task_dir.join("worker-request-1.json.damaged").exists()
        });
        let number = world.one_more_request(&member, &worker);
        world.settle_as_a_look(number, &member, &worker);
        assert!(record_path.exists(), "the record is kept after a later pass");
        assert!(worktree.exists(), "the worktree is kept after a later pass");
        assert!(
            world
                .git_output(&member, &["branch", "--list", &branch])
                .contains(&branch),
            "the branch is kept after a later pass"
        );
        assert!(
            task_dir.join("worker-request-1.json.damaged").exists(),
            "the mismatching pending is kept"
        );
        world.stop();
        world.start();
        let number = world.one_more_request(&member, &worker);
        world.settle_as_a_look(number, &member, &worker);
        assert!(record_path.exists(), "the record is kept after a reload");
        assert!(worktree.exists(), "the worktree is kept after a reload");
        assert!(
            task_dir.join("worker-request-1.json.damaged").exists(),
            "the mismatching pending is kept after a reload"
        );
    }

    #[test]
    fn a_wrong_result_of_a_workspace_request_is_given_once_more_and_the_workspace_is_released_after_the_handoff() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "sleep"),
            ("glm-2", "worker", "sleep,script:guide"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-2"]);
        let (member, second) = (task(&tasks, "claude-1").to_owned(), task(&tasks, "glm-2").to_owned());
        for name in ["claude-1", "inv-opus", "glm-2"] {
            world.ended(task(&tasks, name), 1);
        }
        world.sleeps_on_the_bus("glm-2");
        world.stop();
        // The workspace of a worker that left nothing: its worktree stays at the requester's committed end.
        let worker_task = "task-20261124-090000-cccc";
        let branch = workspace_of(&world, &member, "glm-1", worker_task, false);

        world.start();

        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "glm-2");
        let again = world.told("claude-1", "went once more to the worker glm-2");
        assert!(again.contains("the check `exists docs/guide.md` failed"), "{again}");
        assert_eq!(world.runs(&second), 2, "the second worker served it once");
        assert_eq!(world.file(&member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.brought_over(&member), 1);
        assert_eq!(world.file(&member, "scratch.txt"), "my notes\n");
        let record = world.home.join("tasks").join(&member).join("request-workspace-1.json");
        let worktree = world.home.join("worktrees/example").join(worker_task);
        let pending = world.home.join("tasks").join(worker_task).join("worker-request-1.json");
        eventually("the workspace to be released after the handoff", || {
            !record.exists()
                && !worktree.exists()
                && !pending.exists()
                && world.git_output(&member, &["branch", "--list", &branch]).is_empty()
        });
    }

    #[test]
    fn a_team_without_a_worker_delegates_a_step_to_a_pool_worker() {
        let mut world = World::new(&[
            ("inv-opus", "investigator", "sleep"),
            ("claude-1", "agent", "ask-worker:member,sleep"),
            ("glm-1", "worker", "script:guide"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        std::fs::write(world.worktree(member).join("scratch.txt"), "my notes\n").unwrap();

        world.ended(member, 1);

        world.request_is("1", "done");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.file(member, "scratch.txt"), "my notes\n");
        let message = world.git_output(member, &["log", "-1", "--format=%B"]);
        assert!(message.contains("Write the walkthrough"), "{message}");
        assert!(message.contains("Brought over from"), "{message}");
    }

    #[test]
    fn a_solo_task_delegates_a_step_to_a_pool_worker() {
        let mut world = World::new(&[
            ("claude-1", "agent", "ask-worker:solo,sleep"),
            ("glm-1", "worker", "script:guide"),
        ]);
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("solo", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1"]);
        let member = task(&tasks, "claude-1");
        std::fs::write(world.worktree(member).join("scratch.txt"), "my notes\n").unwrap();

        world.ended(member, 1);

        world.request_is("1", "done");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.file(member, "scratch.txt"), "my notes\n");
        let message = world.git_output(member, &["log", "-1", "--format=%B"]);
        assert!(message.contains("Write the walkthrough"), "{message}");
        assert!(message.contains("Brought over from"), "{message}");
    }

    /// Full specification of an agent for custom pool scenarios.
    struct AgentDef {
        name: &'static str,
        kind: &'static str,
        scenario: &'static str,
        cli: &'static str,
        model: Option<&'static str>,
        capabilities: &'static [&'static str],
    }

    fn init_custom_repo(root: &Path) -> PathBuf {
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
        repo
    }

    impl World {
        fn new_custom(agents: &[AgentDef], limits: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            let (mut normals, mut workers) = (Vec::new(), Vec::new());
            let reviewer_defs: Vec<AgentDef> = REVIEWERS
                .iter()
                .map(|(n, k, s)| AgentDef {
                    name: n,
                    kind: k,
                    scenario: s,
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                })
                .chain(std::iter::once(AgentDef {
                    name: "inv-opus",
                    kind: "investigator",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                }))
                .collect();
            for agent in reviewer_defs.iter().chain(agents) {
                let table = if agent.kind == "investigator" {
                    "investigators"
                } else {
                    "agents"
                };
                let model_line = agent.model.map(|m| format!("model = \"{m}\"\n")).unwrap_or_default();
                let caps_line = if agent.capabilities.is_empty() {
                    String::new()
                } else {
                    let formatted = agent
                        .capabilities
                        .iter()
                        .map(|c| format!("\"{c}\""))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("capabilities = [{formatted}]\n")
                };
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"{cli}\"\n{model_line}{caps_line}args = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \
                     \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    name = agent.name,
                    cli = agent.cli,
                    scenario = agent.scenario,
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
                .unwrap();
                match agent.kind {
                    "agent" => normals.push(format!("\"{}\"", agent.name)),
                    "worker" => workers.push(format!("\"{}\"", agent.name)),
                    _ => {}
                }
            }
            let limits_block = limits.map(|l| format!("[limits]\n{l}\n")).unwrap_or_default();
            write!(
                config,
                "[plan]\nbounces = 0\n[tiers]\nnormal = [{}]\nworkers = [{}]\n{limits_block}[binaries]\nclaude = \"{cli}\"\nopencode = \"{cli}\"\ncodex = \"{cli}\"\nagy = \"{cli}\"\nagentbus = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n",
                normals.join(", "),
                workers.join(", "),
                agentbus_binary().display(),
                cli = script("team-cli.sh").display(),
            )
            .unwrap();
            std::fs::write(home.join("agents.toml"), config).unwrap();
            for dir in ["fake-plans", "fake-requests"] {
                std::fs::create_dir(home.join(dir)).unwrap();
            }
            let repo = init_custom_repo(&root);
            Self {
                _dir: dir,
                home,
                repo,
                daemon: None,
            }
        }
    }

    #[test]
    fn pool_alternates_providers_across_three_requests() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("deepinfra/glm"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/qwen"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script(
            "guide",
            "mkdir -p docs\necho walkthrough >> docs/guide.md\ngit add docs/guide.md\ngit -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'\n",
        );
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);

        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "w-di", "first provider is deepinfra");
        world.feedback_as(member, 1, "used");

        let reply2 = world.delegate_as(member, MEMBER_REQUEST);
        assert!(!answers(&reply2)[0].0, "{reply2}");
        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-or", "second provider is openrouter");
        world.feedback_as(member, 2, "used");

        let reply3 = world.delegate_as(member, MEMBER_REQUEST);
        assert!(!answers(&reply3)[0].0, "{reply3}");
        world.request_is("3", "done");
        assert_eq!(
            world.requests()["3"]["worker"],
            "w-di",
            "third request alternates back to deepinfra"
        );
    }

    #[test]
    fn pool_skips_busy_provider_without_waiting() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-hold",
                    cli: "opencode",
                    model: Some("deepinfra/glm"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/qwen"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);

        world.request_is("1", "running");
        assert_eq!(world.requests()["1"]["worker"], "w-di");

        let reply2 = world.delegate_as(member, MEMBER_REQUEST);
        assert!(!answers(&reply2)[0].0, "{reply2}");

        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-or");
    }

    #[test]
    fn pool_worker_never_double_served_across_teams() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "glm-1",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();

        let tasks1 = world.run(&["claude-1", "inv-opus"]);
        let member1 = task(&tasks1, "claude-1");
        std::fs::create_dir_all(world.worktree(member1).join(".agent")).unwrap();
        std::fs::write(world.worktree(member1).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member1, 1);

        let tasks2 = world.run(&["claude-2", "inv-opus"]);
        let member2 = task(&tasks2, "claude-2");
        std::fs::create_dir_all(world.worktree(member2).join(".agent")).unwrap();
        std::fs::write(world.worktree(member2).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member2, 1);

        // Team 1 delegates step 1.1 -> accepted
        let reply1 = world.delegate_as(member1, MEMBER_REQUEST);
        assert!(!answers(&reply1)[0].0, "{reply1}");

        // Team 2 delegates step 1.1 at the same time with only 1 worker in the pool -> refused as NoFreeWorker
        let reply2 = world.delegate_as(member2, MEMBER_REQUEST);
        let got2 = answers(&reply2);
        assert!(got2[0].0, "second team must be refused: {reply2}");
        assert!(got2[0].1.contains("no worker of the pool is free now"), "{got2:?}");

        // Only request 1 was accepted; worker is never double-served across teams
        assert_eq!(world.requests().len(), 1);
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "glm-1");
    }

    #[test]
    fn pool_worker_run_limit_prevents_delegation() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "glm-1",
                    kind: "worker",
                    scenario: "hold",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            Some("runs_per_worker = 1"),
        );
        world.start();

        let tasks1 = world.run(&["claude-1", "inv-opus"]);
        let member1 = task(&tasks1, "claude-1");
        std::fs::create_dir_all(world.worktree(member1).join(".agent")).unwrap();
        std::fs::write(world.worktree(member1).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member1, 1);

        let tasks2 = world.run(&["claude-2", "inv-opus"]);
        let member2 = task(&tasks2, "claude-2");
        std::fs::create_dir_all(world.worktree(member2).join(".agent")).unwrap();
        std::fs::write(world.worktree(member2).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member2, 1);

        let reply1 = world.delegate_as(member1, MEMBER_REQUEST);
        assert!(!answers(&reply1)[0].0, "{reply1}");

        let reply2 = world.delegate_as(member2, MEMBER_REQUEST);
        let got2 = answers(&reply2);
        assert!(got2[0].0, "second team must be refused: {reply2}");
        assert!(got2[0].1.contains("no worker of the pool is free now"), "{got2:?}");
    }

    #[test]
    fn pool_second_try_picks_different_provider_same_cli() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-done",
                    cli: "opencode",
                    model: Some("deepinfra/glm"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/qwen"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.allow_tries(2);
        world.script("nothing", "true\n");
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);

        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "w-or");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
    }

    #[test]
    fn pool_worker_isolation_omits_team_flag_and_team_prompt() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "glm-1",
                    kind: "worker",
                    scenario: "hold",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            None,
        );
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);

        world.request_is("1", "running");
        let worker_task = world.requests()["1"]["helper_task"].as_str().unwrap().to_owned();

        // 1. Verify mcp.json args omit --team
        let runs_dir = world.home.join("tasks").join(&worker_task).join("runs");
        let mut configs: Vec<PathBuf> = std::fs::read_dir(&runs_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path().join("mcp.json"))
            .filter(|path| path.exists())
            .collect();
        configs.sort();
        let mcp_path = configs.last().unwrap();
        let mcp_text = std::fs::read_to_string(mcp_path).unwrap();
        let mcp_json: serde_json::Value = serde_json::from_str(&mcp_text).unwrap();
        let args = mcp_json["mcpServers"]["agentbus"]["args"].as_array().unwrap();
        let has_team = args.iter().any(|a| a.as_str() == Some("--team"));
        assert!(
            !has_team,
            "pool worker MCP arguments must not include --team: {mcp_text}"
        );

        // 2. Verify prompt omits team context
        let prompt_path = world
            .home
            .join("worktrees/example")
            .join(&worker_task)
            .join(".fake-prompt-1");
        let prompt_text = std::fs::read_to_string(&prompt_path).unwrap();
        assert!(
            !prompt_text.contains("coordinator"),
            "prompt must omit team context: {prompt_text}"
        );
        assert!(
            !prompt_text.contains("The team is"),
            "prompt must omit team context: {prompt_text}"
        );
    }

    #[test]
    fn pool_worker_capabilities_matching() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-rust",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &["build:rust"],
                },
                AgentDef {
                    name: "w-py",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &["build:python"],
                },
            ],
            None,
        );
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);

        // 1. Request requiring "build:rust" -> served by w-rust
        let req_rust = r#"{"goal":"build","workdir":"/tmp","steps":"cargo test","returns":"status","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Write guide","requires":["build:rust"]}"#;
        let reply1 = world.delegate_as(member, req_rust);
        assert!(!answers(&reply1)[0].0, "{reply1}");
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "w-rust");
        world.feedback_as(member, 1, "used");

        // 2. Request requiring "build:python" -> served by w-py
        let req_py = r#"{"goal":"build","workdir":"/tmp","steps":"pytest","returns":"status","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Write guide","requires":["build:python"]}"#;
        let reply2 = world.delegate_as(member, req_py);
        assert!(!answers(&reply2)[0].0, "{reply2}");
        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-py");
        world.feedback_as(member, 2, "used");

        // 3. Request requiring "hw:gpu" -> neither meets it, remains queued
        let req_gpu = r#"{"goal":"train","workdir":"/tmp","steps":"train.py","returns":"model","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Train model","requires":["hw:gpu"]}"#;
        let reply3 = world.delegate_as(member, req_gpu);
        assert!(!answers(&reply3)[0].0, "{reply3}");

        // 4. Request requiring "build:rust" -> served by w-rust
        let req_rust2 = r#"{"goal":"build","workdir":"/tmp","steps":"cargo test","returns":"status","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Write guide again","requires":["build:rust"]}"#;
        let reply4 = world.delegate_as(member, req_rust2);
        assert!(!answers(&reply4)[0].0, "{reply4}");
        world.request_is("4", "done");
        assert_eq!(world.requests()["4"]["worker"], "w-rust");

        // Request 3 ("hw:gpu") was evaluated and skipped because neither worker has "hw:gpu",
        // remaining queued and unassigned while request 4 finished.
        let req3 = &world.requests()["3"];
        assert_eq!(req3["status"], "queued");
        assert!(req3.get("worker").is_none() || req3["worker"].is_null());
    }

    #[test]
    fn pool_cross_team_provider_alternation() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("deepinfra/glm"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/qwen"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script(
            "guide",
            "mkdir -p docs\necho walkthrough >> docs/guide.md\ngit add docs/guide.md\ngit -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'\n",
        );
        world.start();

        let tasks1 = world.run(&["claude-1", "inv-opus"]);
        let member1 = task(&tasks1, "claude-1");
        std::fs::create_dir_all(world.worktree(member1).join(".agent")).unwrap();
        std::fs::write(world.worktree(member1).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member1, 1);

        let tasks2 = world.run(&["claude-2", "inv-opus"]);
        let member2 = task(&tasks2, "claude-2");
        std::fs::create_dir_all(world.worktree(member2).join(".agent")).unwrap();
        std::fs::write(world.worktree(member2).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member2, 1);

        // Team 1 delegates -> first provider is deepinfra
        let reply1 = world.delegate_as(member1, MEMBER_REQUEST);
        assert!(!answers(&reply1)[0].0, "{reply1}");
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "w-di");
        world.feedback_as(member1, 1, "used");

        // Team 2 delegates -> alternates across teams to openrouter
        let reply2 = world.delegate_as(member2, MEMBER_REQUEST);
        assert!(!answers(&reply2)[0].0, "{reply2}");
        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-or");
        world.feedback_as(member2, 2, "used");

        // Team 1 delegates -> alternates back to deepinfra
        let reply3 = world.delegate_as(member1, MEMBER_REQUEST);
        assert!(!answers(&reply3)[0].0, "{reply3}");
        world.request_is("3", "done");
        assert_eq!(world.requests()["3"]["worker"], "w-di");
    }

    #[test]
    fn pool_skips_busy_next_provider_in_three_provider_rotation() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-hold",
                    cli: "opencode",
                    model: Some("deepinfra/glm"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-cl",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/qwen"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script(
            "guide",
            "mkdir -p docs\necho walkthrough >> docs/guide.md\ngit add docs/guide.md\ngit -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'\n",
        );
        world.start();

        let tasks1 = world.run(&["claude-1", "inv-opus"]);
        let member1 = task(&tasks1, "claude-1");
        std::fs::create_dir_all(world.worktree(member1).join(".agent")).unwrap();
        std::fs::write(world.worktree(member1).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member1, 1);

        let tasks2 = world.run(&["claude-2", "inv-opus"]);
        let member2 = task(&tasks2, "claude-2");
        std::fs::create_dir_all(world.worktree(member2).join(".agent")).unwrap();
        std::fs::write(world.worktree(member2).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member2, 1);

        // 1. Team 1 delegates -> takes w-di and holds, making deepinfra provider busy
        let reply1 = world.delegate_as(member1, MEMBER_REQUEST);
        assert!(!answers(&reply1)[0].0, "{reply1}");
        world.request_is("1", "running");
        assert_eq!(world.requests()["1"]["worker"], "w-di");

        // 2. Team 2 delegates -> served by w-cl (claude provider)
        let reply2 = world.delegate_as(member2, MEMBER_REQUEST);
        assert!(!answers(&reply2)[0].0, "{reply2}");
        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-cl");
        world.feedback_as(member2, 2, "used");

        // 3. Team 2 delegates again -> next in rotation after claude is deepinfra,
        // but deepinfra (w-di) is still busy running request 1!
        // Daemon skips deepinfra and serves with openrouter (w-or).
        let reply3 = world.delegate_as(member2, MEMBER_REQUEST);
        assert!(!answers(&reply3)[0].0, "{reply3}");
        world.request_is("3", "done");
        assert_eq!(world.requests()["3"]["worker"], "w-or");
    }

    #[test]
    fn pool_worker_independent_run_limit_selects_next_worker() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-1",
                    kind: "worker",
                    scenario: "hold",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-2",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            Some("runs_per_worker = 1"),
        );
        world.script(
            "guide",
            "mkdir -p docs\necho walkthrough >> docs/guide.md\ngit add docs/guide.md\ngit -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Write the walkthrough'\n",
        );
        world.start();

        let tasks1 = world.run(&["claude-1", "inv-opus"]);
        let member1 = task(&tasks1, "claude-1");
        std::fs::create_dir_all(world.worktree(member1).join(".agent")).unwrap();
        std::fs::write(world.worktree(member1).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member1, 1);

        let tasks2 = world.run(&["claude-2", "inv-opus"]);
        let member2 = task(&tasks2, "claude-2");
        std::fs::create_dir_all(world.worktree(member2).join(".agent")).unwrap();
        std::fs::write(world.worktree(member2).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member2, 1);

        // 1. Team 1 delegates -> w-1 takes request 1 and holds (running, at limit 1)
        let reply1 = world.delegate_as(member1, MEMBER_REQUEST);
        assert!(!answers(&reply1)[0].0, "{reply1}");
        world.request_is("1", "running");
        assert_eq!(world.requests()["1"]["worker"], "w-1");

        // 2. Team 2 delegates -> w-1 is at run limit 1, so daemon selects w-2
        let reply2 = world.delegate_as(member2, MEMBER_REQUEST);
        assert!(!answers(&reply2)[0].0, "{reply2}");
        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-2");
        world.feedback_as(member2, 2, "used");
    }

    #[test]
    fn pool_unsettled_result_prevents_worker_reuse_until_settled() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-1",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();

        let tasks1 = world.run(&["claude-1", "inv-opus"]);
        let member1 = task(&tasks1, "claude-1");
        std::fs::create_dir_all(world.worktree(member1).join(".agent")).unwrap();
        std::fs::write(world.worktree(member1).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member1, 1);

        let tasks2 = world.run(&["claude-2", "inv-opus"]);
        let member2 = task(&tasks2, "claude-2");
        std::fs::create_dir_all(world.worktree(member2).join(".agent")).unwrap();
        std::fs::write(world.worktree(member2).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member2, 1);

        // 1. Team 1 delegates with a held check so the check starts and holds (unsettled result)
        let mut req: serde_json::Value = serde_json::from_str(MEMBER_REQUEST).unwrap();
        req["check"] = serde_json::Value::String(HELD_CHECK.to_owned());
        let reply1 = world.delegate_as(member1, &req.to_string());
        assert!(!answers(&reply1)[0].0, "{reply1}");

        eventually("the worker run to start", || {
            world.requests().get("1").is_some_and(|r| r["status"] == "running")
        });
        let worker_task = world.requests()["1"]["helper_task"].as_str().unwrap().to_owned();
        let started = world
            .home
            .join("worktrees/example")
            .join(&worker_task)
            .join(".fake-check-started");
        eventually("the check to start", || started.exists());

        // 2. While w-1's check is held (result is unsettled), Team 2 delegates -> refused because w-1 has unsettled result
        let reply2 = world.delegate_as(member2, MEMBER_REQUEST);
        let got2 = answers(&reply2);
        assert!(got2[0].0, "must be refused while result is unsettled: {reply2}");
        assert!(got2[0].1.contains("no worker of the pool is free now"), "{got2:?}");

        // 3. Allow check to pass and settle
        let go = world
            .home
            .join("worktrees/example")
            .join(&worker_task)
            .join(".fake-check-go");
        std::fs::write(go, "").unwrap();
        world.request_is("1", "done");
        world.feedback_as(member1, 1, "used");

        eventually("workspace of request 1 to be released", || {
            !world
                .home
                .join("tasks")
                .join(member1)
                .join("request-workspace-1.json")
                .exists()
        });

        // 4. Team 2 delegates again -> now accepted and served by w-1
        let reply2_again = world.delegate_as(member2, MEMBER_REQUEST);
        assert!(!answers(&reply2_again)[0].0, "{reply2_again}");
        world.request_is("2", "done");
        assert_eq!(world.requests()["2"]["worker"], "w-1");
    }

    #[test]
    fn pool_k2_two_queued_requests_with_one_free_worker_starts_only_one_run() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-1",
                    kind: "worker",
                    scenario: "hold",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            None,
        );
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        std::fs::create_dir_all(world.worktree(member).join(".agent")).unwrap();
        std::fs::write(world.worktree(member).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        world.ended(member, 1);

        // Stop daemon so we can set up two queued requests in bus state
        world.stop();

        // Delegate first request directly to bus state
        let reply1 = world.delegate_as(member, MEMBER_REQUEST);
        assert!(!answers(&reply1)[0].0, "{reply1}");

        // Duplicate request 1 as request 2 in bus/state.json to simulate 2 simultaneously queued requests
        let bus_state_path = world.home.join("bus/state.json");
        let mut bus_state: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&bus_state_path).unwrap()).unwrap();
        let h1 = bus_state["helpers"]["1"].clone();
        let mut h2 = h1.clone();
        h2["id"] = serde_json::json!(2);
        bus_state["helpers"]["2"] = h2;
        std::fs::write(&bus_state_path, serde_json::to_string_pretty(&bus_state).unwrap()).unwrap();

        // Restart daemon: serve_pool_requests encounters both queued requests in a single tick
        world.start();

        // Request 1 is dispatched to w-1 and starts running
        world.request_is("1", "running");
        assert_eq!(world.requests()["1"]["worker"], "w-1");

        // Request 2 must NOT be double-dispatched to w-1: it must remain queued and unassigned
        let req2 = &world.requests()["2"];
        assert_eq!(req2["status"], "queued");
        assert!(req2.get("worker").is_none() || req2["worker"].is_null());
    }

    #[test]
    fn pool_second_try_incompatible_alternate_provider_picks_compatible_same_provider() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-cl-1",
                    kind: "worker",
                    scenario: "done",
                    cli: "claude",
                    model: None,
                    capabilities: &["build:rust"],
                },
                AgentDef {
                    name: "w-gemini",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("google/gemini-2.5-flash"),
                    capabilities: &["build:python"],
                },
                AgentDef {
                    name: "w-cl-2",
                    kind: "worker",
                    scenario: "script:guide",
                    cli: "claude",
                    model: None,
                    capabilities: &["build:rust"],
                },
            ],
            None,
        );
        world.allow_tries(2);
        world.script("guide", &commit_script("the walkthrough", &[]));
        let req_rust = r#"{"goal":"build","workdir":"/tmp","steps":"cargo test","returns":"status","for":"1.1","files":"docs/guide.md","check":"exists docs/guide.md","do":"Write guide","requires":["build:rust"]}"#;
        world.asks("member", Some(MEMBER_PLAN), &[req_rust]);
        world.start();

        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);

        world.request_is("1", "done");
        // w-cl-1 failed the check; alternate provider "google" (w-gemini) lacks "build:rust";
        // second try falls back to compatible worker of same provider "claude" (w-cl-2).
        assert_eq!(world.requests()["1"]["worker"], "w-cl-2");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
    }
    #[test]
    fn review_legacy_crew_preserves_configured_provider_order() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "claude-2",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di-1",
                    kind: "worker",
                    scenario: "hold",
                    cli: "opencode",
                    model: Some("deepinfra/first"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/model"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di-2",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("deepinfra/second"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();
        let legacy = world.run(&["claude-1", "w-di-1", "inv-opus"]);
        world.ended(task(&legacy, "claude-1"), 1);
        let tasks = world.run(&["claude-2", "inv-opus"]);
        let member = task(&tasks, "claude-2");
        world.ended(member, 1);
        std::fs::create_dir_all(world.worktree(member).join(".agent")).unwrap();
        std::fs::write(world.worktree(member).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        let reply = world.delegate_as(member, MEMBER_REQUEST);
        assert!(!answers(&reply)[0].0, "{reply}");
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "w-di-2");
    }

    #[test]
    fn review_single_worker_receives_its_second_try() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "only",
                    kind: "worker",
                    scenario: "script:retry",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
            ],
            None,
        );
        world.allow_tries(2);
        // Retry selection is independent of the discarded worktree of the wrong first result.
        world.script("retry", &format!("if test -f \"$AGENTS_HOME/first-worker-try\"; then\n{}\nelse\ntouch \"$AGENTS_HOME/first-worker-try\"\nmkdir -p docs\necho first > docs/guide.md\ngit add docs/guide.md\ngit -c user.name=Worker -c user.email=worker@example.com commit -q -m 'Record first try'\nfi\n", commit_script("the walkthrough", &[])));
        let plan = MEMBER_PLAN.replace("exists docs/guide.md", "grep -q walkthrough docs/guide.md");
        let request = MEMBER_REQUEST.replace("exists docs/guide.md", "grep -q walkthrough docs/guide.md");
        world.asks("member", Some(&plan), &[&request]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "only");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
    }
    #[test]
    fn failed_pool_preparation_does_not_advance_the_provider_turn() {
        use std::os::unix::fs::PermissionsExt;
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("deepinfra/model"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/model"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);
        let hooks = world.home.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("post-checkout");
        std::fs::write(
            &hook,
            r#"#!/bin/sh
marker="$(dirname "$0")/failed"
if ! test -e "$marker"; then
    touch "$marker"
    exit 1
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(
            &world.worktree(member),
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
        std::fs::create_dir_all(world.worktree(member).join(".agent")).unwrap();
        std::fs::write(world.worktree(member).join(".agent/plan.md"), MEMBER_PLAN).unwrap();
        let reply = world.delegate_as(member, MEMBER_REQUEST);
        assert!(!answers(&reply)[0].0, "{reply}");
        world.request_is("1", "done");
        assert!(hooks.join("failed").exists(), "the preparation failure was injected");
        assert_eq!(world.requests()["1"]["worker"], "w-di");
    }
    #[test]
    fn declared_failure_after_committing_is_retried_on_another_provider() {
        let mut world = World::new_custom(
            &[
                AgentDef {
                    name: "claude-1",
                    kind: "agent",
                    scenario: "ask-worker:member,sleep",
                    cli: "claude",
                    model: None,
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-di",
                    kind: "worker",
                    scenario: "opencode-declared:wrong",
                    cli: "opencode",
                    model: Some("deepinfra/glm"),
                    capabilities: &[],
                },
                AgentDef {
                    name: "w-or",
                    kind: "worker",
                    scenario: "opencode-script:guide",
                    cli: "opencode",
                    model: Some("openrouter/qwen"),
                    capabilities: &[],
                },
            ],
            None,
        );
        world.allow_tries(2);
        world.script("wrong", &commit_script("unfinished guide", &[]));
        world.script("guide", &commit_script("the walkthrough", &[]));
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus"]);
        let member = task(&tasks, "claude-1");
        world.ended(member, 1);
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "w-or");
        assert_eq!(world.file(member, "docs/guide.md"), "the walkthrough\n");
        assert_eq!(world.count_told("claude-1", "did step 1.1 of your plan"), 1);
        world.told("claude-1", "w-or");
    }
    #[test]
    fn legacy_crew_declared_failure_uses_another_provider_and_one_notice() {
        let (world, tasks) = second_try_world(
            &[
                ("glm-1", "sleep,script-declared:guide"),
                ("glm-2", "sleep,script:guide"),
            ],
            Some(2),
        );
        world.request_is("1", "done");
        assert_eq!(world.requests()["1"]["worker"], "glm-2");
        assert_eq!(
            world.file(task(&tasks, "claude-1"), "docs/guide.md"),
            "the walkthrough\n"
        );
        assert_eq!(world.count_told("claude-1", "did step 1.1 of your plan"), 1);
    }
}
