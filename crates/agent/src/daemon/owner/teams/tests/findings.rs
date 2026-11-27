//! A pooled worker sees findings on its step files before its first run.

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use agent_core::config::{AbsolutePath, Program};
use agent_core::ids::{AgentName, TaskId, TeamId};
use agent_core::protocol::{AgentList, Client, Command, Reply, RunRequest};
use agent_core::state::{MergeMode, State, TaskListText, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{AgentBus, DelegationRequest, DelegationStatus, RequestId};
use agent_runtime::clock::SharedClock;
use agent_runtime::run::RunTimings;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::client::{ClientError, Connection};
use crate::daemon::owner::fixture::agentbus_binary;
use crate::daemon::{DaemonError, DaemonSetup, Home, Timings, run};
use crate::testing::ManualClock;

const PATIENCE: Duration = Duration::from_secs(60);
const PLAIN: &str = "Fix the typo in the README.\n";

enum Kind {
    Normal,
    Investigator,
    Worker,
}

struct Agent {
    name: &'static str,
    kind: Kind,
    scenario: &'static str,
}

fn worker(name: &'static str, scenario: &'static str) -> Agent {
    Agent {
        name,
        kind: Kind::Worker,
        scenario,
    }
}

fn investigator(name: &'static str, scenario: &'static str) -> Agent {
    Agent {
        name,
        kind: Kind::Investigator,
        scenario,
    }
}

fn normal(name: &'static str, scenario: &'static str) -> Agent {
    Agent {
        name,
        kind: Kind::Normal,
        scenario,
    }
}

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

fn fixture_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Process::new("git")
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
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn timings() -> Timings {
    Timings {
        tick: Duration::from_millis(25),
        run: RunTimings {
            poll: Duration::from_millis(20),
            hang: Duration::from_secs(20),
            stop_grace: Duration::from_secs(5),
        },
    }
}

fn config(agents: &[Agent], agentbus: &Path, home: &Home) -> String {
    let mut text = String::new();
    let mut normals = Vec::new();
    let mut workers = Vec::new();
    for agent in agents {
        let table = match agent.kind {
            Kind::Investigator => "investigators",
            Kind::Normal | Kind::Worker => "agents",
        };
        write!(
            text,
            "[{table}.{}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{}\", \"--as\", \"{}\", \"--bus-bin\", \"{}\", \
             \"--bus-dir\", \"{}/bus\", \"--home\", \"{}\"]\n",
            agent.name,
            agent.scenario,
            agent.name,
            agentbus_binary().display(),
            home.root().display(),
            home.root().display(),
        )
        .unwrap();
        match agent.kind {
            Kind::Normal => normals.push(format!("\"{}\"", agent.name)),
            Kind::Worker => workers.push(format!("\"{}\"", agent.name)),
            Kind::Investigator => {}
        }
    }
    write!(
        text,
        "[plan]\nbounces = 0\n[tiers]\nnormal = [{}]\nworkers = [{}]\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
         [watchdog]\nhang_minutes = 1\nhang_strikes = 3\n\
         [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
        normals.join(", "),
        workers.join(", "),
        fixture_script("team-cli.sh").display(),
        agentbus.display(),
    )
    .unwrap();
    text
}

struct Fixture {
    _dir: tempfile::TempDir,
    home: Home,
    repo: PathBuf,
    clock: ManualClock,
    shutdown: CancellationToken,
    daemon: Option<JoinHandle<Result<(), DaemonError>>>,
}

impl Fixture {
    fn new(agents: &[Agent]) -> Self {
        drop(
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_test_writer()
                .try_init(),
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = Home::new(&abs(&root.join("home")));
        std::fs::create_dir_all(home.root()).unwrap();
        std::fs::write(home.config_file(), config(agents, agentbus_binary(), &home)).unwrap();
        std::fs::write(home.root().join("real-agentbus"), agentbus_binary().to_str().unwrap()).unwrap();
        let repo = root.join("example");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "Hello\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-q", "-m", "First"]);
        Self {
            _dir: dir,
            home,
            repo,
            clock: ManualClock::new(Timestamp::now(&agent_runtime::clock::SystemClock).unwrap()),
            shutdown: CancellationToken::new(),
            daemon: None,
        }
    }

    fn setup(&self) -> DaemonSetup {
        DaemonSetup {
            local_offset: time::UtcOffset::UTC,
            home: self.home.clone(),
            prompts_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            clock: SharedClock::new(self.clock.clone()),
            zone: agent_core::local_time::Zone::Fixed(time::UtcOffset::UTC),
            timings: Some(timings()),
            agent_binary_dir: None,
        }
    }

    async fn start(&mut self) {
        self.shutdown = CancellationToken::new();
        let socket = self.home.socket_file();
        let deadline = Instant::now() + PATIENCE;
        'attempts: loop {
            let daemon = tokio::spawn(run(self.setup(), self.shutdown.clone()));
            while !socket.exists() {
                if daemon.is_finished() {
                    match daemon.await.unwrap() {
                        Err(DaemonError::AlreadyRunning { .. }) if Instant::now() < deadline => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue 'attempts;
                        }
                        ended => panic!("the daemon ended at its start: {ended:?}"),
                    }
                }
                assert!(Instant::now() < deadline, "timed out waiting for the socket");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.daemon = Some(daemon);
            return;
        }
    }

    async fn stop(&mut self) -> Result<(), DaemonError> {
        self.shutdown.cancel();
        let daemon = self.daemon.take().unwrap();
        tokio::time::timeout(PATIENCE, daemon).await.unwrap().unwrap()
    }

    async fn connect(&self) -> Connection {
        let socket = self.home.socket_file();
        let deadline = Instant::now() + PATIENCE;
        loop {
            match Connection::connect(&socket).await {
                Ok(connection) => return connection,
                Err(ClientError::NotRunning { .. }) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("cannot connect to the daemon: {error}"),
            }
        }
    }

    async fn start_team(&self, agents: &[&str], list: &str) -> (TeamId, Vec<TaskId>) {
        let names: Vec<AgentName> = agents.iter().map(|name| (*name).parse().unwrap()).collect();
        let request = RunRequest {
            allow_unpushed_base: false,
            repo: "example".parse().unwrap(),
            checkout: abs(&self.repo),
            agents: AgentList::try_from(names).unwrap(),
            task_list: TaskListText::new(list.to_owned()).unwrap(),
            coordinator: None,
            merge: MergeMode::Autopilot,
        };
        let reply = self
            .connect()
            .await
            .request(Client::Cli, Command::Run(request))
            .await
            .unwrap();
        let Reply::Started { team, tasks, .. } = reply else {
            panic!("a team was expected to start: {reply:?}")
        };
        (team, tasks)
    }

    fn bus(&self) -> AgentBus {
        AgentBus::new(
            Program::from_str(agentbus_binary().to_str().unwrap()).unwrap(),
            abs(self.home.root()),
        )
    }

    async fn status_is(&self, task: &TaskId, status: TaskStatus) -> State {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Ok(text) = std::fs::read_to_string(self.home.state_file())
                && let Ok(state) = State::from_json(&text)
                && state
                    .find_task(task)
                    .is_some_and(|found| found.member.status() == status)
            {
                return state;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {task} to be {status}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn request_with(&self, id: RequestId, status: DelegationStatus) -> DelegationRequest {
        let bus = self.bus();
        let deadline = Instant::now() + PATIENCE;
        loop {
            let request = bus.helper_get(id).await.unwrap();
            if request.status == status {
                return request;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the request to be {status}: {request:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn worktree(&self, task: &TaskId) -> PathBuf {
        self.home.worktree_dir(&"example".parse().unwrap(), task)
    }
}

fn base_origin(f: &Fixture) -> PathBuf {
    let origin = f
        .repo
        .parent()
        .expect("fixture repository has a parent")
        .join("origin.git");
    git(
        f.repo.parent().expect("fixture repository has a parent"),
        &["init", "--bare", "-q", "-b", "main", origin.to_str().unwrap()],
    );
    git(&f.repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&f.repo, &["push", "-q", "origin", "main"]);
    origin
}

async fn first_run(f: &Fixture, task: &TaskId) -> PathBuf {
    let deadline = Instant::now() + PATIENCE;
    loop {
        tokio::task::yield_now().await;
        let runs = f.home.runs_dir(task);
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&runs)
            .map(|runs| runs.map(|run| run.unwrap().path()).collect())
            .unwrap_or_default();
        dirs.sort();
        if let Some(first) = dirs.first() {
            return first.clone();
        }
        assert!(Instant::now() < deadline, "no run of {task} started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_file(path: PathBuf) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        match std::fs::read_to_string(&path) {
            Ok(text) => return text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!("cannot read {path:?}: {error}"),
        }
    }
}

#[tokio::test]
async fn a_pooled_worker_sees_findings_on_its_step_files_and_records_them() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "hold"),
    ]);
    std::fs::create_dir(f.repo.join("src")).unwrap();
    std::fs::write(f.repo.join("src/lib.rs"), "base").unwrap();
    git(&f.repo, &["add", "src/lib.rs"]);
    git(&f.repo, &["commit", "-m", "Source"]);
    base_origin(&f);
    f.start().await;
    let (_, tasks) = f.start_team(&["inv-opus", "claude-1"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    let directory = f.home.root().join("bus/notebooks/example");
    std::fs::create_dir_all(&directory).unwrap();
    let finding = serde_json::json!({"op":"add","id":"F-1","ts":"2099-01-01T00:00:00Z","author":"other","task":"task-20261120-000000-aaaa","kind":"trap","paths":["src/lib.rs"],"text":"Use the checked helper"});
    std::fs::write(directory.join("findings.jsonl"), format!("{finding}\n")).unwrap();
    let accept = f.home.root().join("accepting-agent");
    std::fs::write(&accept, "#!/bin/sh\nprintf 'ACCEPTED\\nStep 1.1 Write it\\nRequest files: src/lib.rs\\nRequest check: exists src/lib.rs\\nDo: Write it.\\n'\n").unwrap();
    std::fs::set_permissions(&accept, std::fs::Permissions::from_mode(0o700)).unwrap();
    let plan = f.home.root().join("plan.md");
    std::fs::write(&plan, "1. Work\n[>] 1.1 Write it\n").unwrap();
    let task = &tasks[1];
    let worktree = f.worktree(task);
    let call = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delegate","arguments":{"goal":"write it","workdir":worktree,"steps":"write it","returns":"the file","for":"1.1","files":"src/lib.rs","check":"exists src/lib.rs","do":"Write it."}}});
    let mut child = Process::new(agentbus_binary())
        .args(["serve", "--agent", "claude-1", "--task", task.as_str(), "--dir"])
        .arg(f.home.root().join("bus"))
        .arg("--home")
        .arg(f.home.root())
        .args(["--repo", "example", "--branch"])
        .arg(format!("agents/claude-1/{task}"))
        .arg("--worktree")
        .arg(&worktree)
        .arg("--plan")
        .arg(plan)
        .arg("--agent-cmd")
        .arg(accept)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{call}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let answer = String::from_utf8(output.stdout).unwrap();
    assert!(answer.contains("Request H1 accepted"), "{answer}");
    let request = f
        .request_with(RequestId::new(std::num::NonZeroU64::MIN), DelegationStatus::Running)
        .await;
    let task = request.helper_task.unwrap();
    first_run(&f, &task).await;
    let prompt = wait_for_file(f.home.session_prompt_file(&task)).await;
    f.stop().await.unwrap();
    assert!(
        prompt.contains("- F-1 [trap] src/lib.rs — Use the checked helper"),
        "{prompt}"
    );
    let shown: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.home.task_dir(&task).join("findings-shown.json")).unwrap()).unwrap();
    assert_eq!(shown, serde_json::json!({"shown":["F-1"]}));
}
