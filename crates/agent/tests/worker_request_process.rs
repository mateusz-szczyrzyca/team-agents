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
            ("inv-opus", "investigator", "ask-worker:coordinator,sleep"),
            ("claude-1", "agent", "ask-worker:member,sleep"),
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
            requests.len() == 2 && requests.values().all(|request| request["status"] == "done")
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
            ("claude-1", "agent", "ask-worker:member,sleep"),
            ("glm-1", "worker", "hold,done:Result: done."),
        ]);
        world.asks("member", Some(MEMBER_PLAN), &[MEMBER_REQUEST]);
        world.start();
        let tasks = world.run(&["claude-1", "inv-opus", "glm-1"]);
        let glm_1 = task(&tasks, "glm-1");
        world.ended(task(&tasks, "claude-1"), 1);
        let got = answers(&world.file(task(&tasks, "claude-1"), ".fake-delegate-1"));
        assert!(!got[0].0, "{got:?}");
        assert_eq!(
            world.requests()["1"]["status"],
            "queued",
            "the worker still holds its first run"
        );

        world.stop();
        std::fs::write(world.worktree(glm_1).join(".fake-continue"), "").unwrap();
        world.start();

        eventually("the request to be done", || world.requests()["1"]["status"] == "done");
        let runs = world.runs(glm_1);
        assert_eq!(
            request_prompts(&world, glm_1, 1, runs).len(),
            1,
            "the request is served once"
        );
        assert_eq!(world.requests().len(), 1);
    }
}
