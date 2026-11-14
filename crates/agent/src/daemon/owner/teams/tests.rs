//! The daemon with teams: a `run` request, the team's life with a stand-in for the `claude` CLI, the real `agentbus`
//! (built from `agentbus/`) and a real scratch repository, all in a temporary home.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use agent_core::config::{AbsolutePath, Program};
use agent_core::ids::{AgentName, TaskId, TeamId};
use agent_core::protocol::{AgentList, Client, Command, FailureKind, Reply, RunRequest, StatusRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, PrNumber, PullRequest, State, TaskListText, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{
    AgentBus, Delegation, DelegationRequest, DelegationStatus, MemberState, Outgoing, Recipient, RequestId,
};
use agent_runtime::clock::SharedClock;
use agent_runtime::run::RunTimings;
use agent_runtime::store::Home;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::client::{ClientError, Connection};
use crate::daemon::{DaemonError, DaemonSetup, Timings, run};
use crate::testing::ManualClock;

/// The longest any wait of these tests lasts.
const PATIENCE: Duration = Duration::from_secs(60);

/// A list of two features that do not wait for each other, with one item each.
const PARALLEL: &str = "\
Build the things.

1. First
[ ] 1.1 One
    Do one.

2. Second
[ ] 2.1 Two
    Do two.
";

/// A list of one feature with two items: the second starts when the first is merged.
const CHAIN: &str = "\
Build the thing.

1. First
[ ] 1.1 One
    Do one.
[ ] 1.2 Two
    Do two.
";

/// A list whose only item is marked done.
const ALL_DONE: &str = "\
Build the thing.

1. First
[x] 1.1 One
    Done already.
";

/// A list without features: one piece of work for the lead.
const PLAIN: &str = "Fix the typo in the README.\n";

/// The `agentbus` built from `agentbus/`, once for all the tests of this file.
fn agentbus_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus");
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        std::fs::create_dir_all(&target).unwrap();
        let binary = target.join("agentbus-under-test-of-agent");
        let built = Process::new("go")
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

/// A fixture script of the `agent` crate.
fn fixture_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

/// Runs git for the setup and the checks of a test and returns its trimmed output.
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
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
}

/// How an agent of the configuration is made.
enum Kind {
    Normal,
    Worker,
    Investigator,
}

/// An agent of the configuration and what its stand-in CLI does.
struct Agent {
    name: &'static str,
    kind: Kind,
    scenario: &'static str,
}

fn normal(name: &'static str, scenario: &'static str) -> Agent {
    Agent {
        name,
        kind: Kind::Normal,
        scenario,
    }
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

/// The configuration of a home with these agents, one repository `example` and the stand-in CLI for every agent.
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
        "[tiers]\nnormal = [{}]\nworkers = [{}]\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n\
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

/// A daemon in a temporary home, with a scratch repository `example` that has one commit on `main`.
struct Fixture {
    _dir: tempfile::TempDir,
    home: Home,
    repo: PathBuf,
    clock: ManualClock,
    shutdown: CancellationToken,
    daemon: Option<JoinHandle<Result<(), DaemonError>>>,
}

impl Fixture {
    /// A home with these agents; the daemon is not started.
    fn new(agents: &[Agent]) -> Self {
        Self::with_agentbus(agents, agentbus_binary())
    }

    /// The same, with `agentbus` as the program of the bus.
    fn with_agentbus(agents: &[Agent], agentbus: &Path) -> Self {
        // The daemon's log goes to the output of the test, which the harness shows when the test fails.
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
        std::fs::write(home.config_file(), config(agents, agentbus, &home)).unwrap();
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
            home: self.home.clone(),
            prompts_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            clock: SharedClock::new(self.clock.clone()),
            local_offset: time::UtcOffset::UTC,
            timings: Some(timings()),
        }
    }

    /// Starts the daemon and waits until its socket is there. Another test of this binary may be starting a process:
    /// a child holds the open file description of the lock of a daemon that has just stopped from its fork to its
    /// exec, so a daemon that finds the home locked is started again for a few milliseconds more.
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

    /// Stops the daemon and returns how it ended.
    async fn stop(&mut self) -> Result<(), DaemonError> {
        self.shutdown.cancel();
        let daemon = self.daemon.take().unwrap();
        tokio::time::timeout(PATIENCE, daemon).await.unwrap().unwrap()
    }

    /// Asks the daemon for its status and waits for the answer. The owner answers between two turns of its loop, and its
    /// first turn looks at every team: when the answer is there, the daemon has looked at the teams since its start.
    async fn answered(&self) {
        let reply = self
            .connect()
            .await
            .request(Client::Cli, Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
    }

    async fn connect(&self) -> Connection {
        Connection::connect(&self.home.socket_file()).await.unwrap()
    }

    /// Asks the daemon to run `agents` on `list` in the scratch repository.
    async fn run(&self, agents: &[&str], list: &str, coordinator: Option<&str>) -> Result<Reply, ClientError> {
        self.run_request(self.request(agents, list, coordinator)).await
    }

    fn request(&self, agents: &[&str], list: &str, coordinator: Option<&str>) -> RunRequest {
        RunRequest {
            repo: "example".parse().unwrap(),
            checkout: abs(&self.repo),
            agents: AgentList::try_from(agents.iter().map(|name| agent(name)).collect::<Vec<_>>()).unwrap(),
            task_list: TaskListText::new(list.to_owned()).unwrap(),
            coordinator: coordinator.map(agent),
            merge: MergeMode::Autopilot,
        }
    }

    async fn run_request(&self, request: RunRequest) -> Result<Reply, ClientError> {
        self.connect().await.request(Client::Cli, Command::Run(request)).await
    }

    /// Starts a team and returns its ID and tasks.
    async fn start_team(&self, agents: &[&str], list: &str) -> (TeamId, Vec<TaskId>) {
        let reply = self.run(agents, list, None).await.unwrap();
        let Reply::Started { team, tasks } = reply else {
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

    /// The state as saved.
    fn state(&self) -> State {
        // Nothing is saved until a team is made.
        match std::fs::read_to_string(self.home.state_file()) {
            Ok(text) => State::from_json(&text).unwrap(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => State::new(),
            Err(error) => panic!("{error}"),
        }
    }

    /// Waits until the saved state satisfies `ready`.
    async fn state_where(&self, what: &str, ready: impl Fn(&State) -> bool) -> State {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Ok(text) = std::fs::read_to_string(self.home.state_file())
                && let Ok(state) = State::from_json(&text)
                && ready(&state)
            {
                return state;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until the task has the status.
    async fn status_is(&self, task: &TaskId, status: TaskStatus) -> State {
        self.state_where(&format!("{task} to be {status}"), |state| {
            state
                .find_task(task)
                .is_some_and(|found| found.member.status() == status)
        })
        .await
    }

    /// Waits until the bus has the member in the state.
    async fn bus_state_is(&self, team: &TeamId, member: &str, state: MemberState) {
        let bus = self.bus();
        let deadline = Instant::now() + PATIENCE;
        loop {
            let found = bus.team_get(team).await.unwrap().members[&agent(member)].state;
            if found == state {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {member} to be {state} on the bus; it is {found}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until the request has the status, and returns it.
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

    /// The prompt the task's `n`-th run was given, once it has been. The script logs the run after it has written the
    /// prompt, so a run that is logged has its whole prompt in its file.
    async fn prompt(&self, task: &TaskId, n: u32) -> String {
        let log = self.worktree(task).join(".fake-log");
        let wanted = usize::try_from(n).unwrap();
        self.wait_for(&format!("run {n} of {task} to be logged"), || {
            std::fs::read_to_string(&log).is_ok_and(|text| text.lines().count() >= wanted)
        })
        .await;
        std::fs::read_to_string(self.worktree(task).join(format!(".fake-prompt-{n}"))).unwrap()
    }

    /// How many runs the task has had.
    fn runs(&self, task: &TaskId) -> u32 {
        std::fs::read_to_string(self.worktree(task).join(".fake-runs")).map_or(0, |text| text.trim().parse().unwrap())
    }

    async fn wait_for(&self, what: &str, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}\nmessages: {}\nbus: {}",
                self.messages(),
                std::fs::read_to_string(self.home.root().join("bus/state.json")).unwrap_or_default()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The notifications written so far.
    fn notifications(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.home.notifications_dir()) else {
            return Vec::new();
        };
        // A notification is written to a temporary file and renamed: only the finished ones count.
        let mut texts: Vec<String> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_none_or(|extension| extension != "tmp"))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect();
        texts.sort();
        texts
    }

    /// What the bus holds as messages, a line each.
    fn messages(&self) -> String {
        std::fs::read_to_string(self.home.root().join("bus/messages.jsonl")).unwrap_or_default()
    }

    /// The branches of the scratch repository.
    fn branches(&self) -> Vec<String> {
        git(&self.repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn task_status(state: &State, task: &TaskId) -> TaskStatus {
    state.find_task(task).unwrap().member.status()
}

/// The kind of a refusal.
fn refused(reply: Result<Reply, ClientError>) -> FailureKind {
    match reply {
        Err(ClientError::Refused(failure)) => failure.kind,
        other => panic!("a refusal was expected: {other:?}"),
    }
}

// ---- a task alone ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_solo_task_is_started_runs_to_done_and_its_team_is_closed_and_told_to_the_human() {
    let mut f = Fixture::new(&[
        normal("claude-1", "done"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["claude-1"], PLAIN).await;

    assert_eq!(tasks.len(), 1);
    let state = f.status_is(&tasks[0], TaskStatus::Done).await;
    assert_eq!(state.teams().len(), 1);
    assert_eq!(state.teams()[0].id(), &team);
    f.wait_for("the notification that the team is finished", || {
        f.notifications().iter().any(|text| text.contains("is finished"))
    })
    .await;
    f.stop().await.unwrap();
    let notice = f
        .notifications()
        .into_iter()
        .find(|text| text.contains("is finished"))
        .unwrap();
    assert!(notice.starts_with(&format!("Team {team} is finished\n")), "{notice}");
}

#[tokio::test]
async fn a_solo_task_gets_a_worktree_on_a_branch_of_its_own_and_its_prompt_has_the_brief() {
    let mut f = Fixture::new(&[
        normal("claude-1", "done"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
    ]);
    f.start().await;

    let (_, tasks) = f.start_team(&["claude-1"], PLAIN).await;
    let prompt = f.prompt(&tasks[0], 1).await;
    f.status_is(&tasks[0], TaskStatus::Done).await;
    f.stop().await.unwrap();

    let worktree = f.worktree(&tasks[0]);
    assert!(worktree.is_dir());
    let branch = format!("agents/claude-1/{}", tasks[0]);
    assert!(f.branches().contains(&branch), "{:?}", f.branches());
    assert_eq!(git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    assert!(prompt.contains("You are agent **claude-1**"), "{prompt}");
    assert!(prompt.contains("Fix the typo in the README."), "{prompt}");
    assert!(prompt.contains("Your role: solo"), "{prompt}");
}

#[tokio::test]
async fn a_solo_task_with_items_starts_the_first_item_on_its_own_branch_and_goes_on_waiting_for_the_next() {
    let mut f = Fixture::new(&[
        normal("claude-1", "done"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["claude-1"], CHAIN).await;
    let prompt = f.prompt(&tasks[0], 1).await;
    let state = f.status_is(&tasks[0], TaskStatus::Waiting).await;
    f.stop().await.unwrap();

    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let progress = state.team(&team).unwrap().items()[0].phase().clone();
    assert!(
        matches!(&progress, ItemPhase::InProgress { assignee, branch: b } if assignee.as_str() == "claude-1" && b.as_str() == branch),
        "{progress:?}"
    );
    assert_eq!(
        git(&f.worktree(&tasks[0]), &["rev-parse", "--abbrev-ref", "HEAD"]),
        branch
    );
    assert!(prompt.contains("## Your item: 1.1 — One"), "{prompt}");
    assert!(
        f.notifications().iter().all(|text| !text.contains("is finished")),
        "the team has items left"
    );
}

// ---- a team ------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_team_starts_with_a_worktree_for_every_task_and_its_team_on_the_bus() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        worker("glm-1", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f
        .start_team(&["inv-opus", "claude-1", "claude-2", "glm-1"], PARALLEL)
        .await;

    assert_eq!(tasks.len(), 4);
    for task in &tasks {
        assert!(f.worktree(task).is_dir(), "{task}");
    }
    let bus = f.bus().team_get(&team).await.unwrap();
    assert_eq!(bus.lead, Some(agent("inv-opus")));
    let names: Vec<String> = bus.members.keys().map(ToString::to_string).collect();
    assert_eq!(names, ["claude-1", "claude-2", "glm-1", "inv-opus"]);
    assert_eq!(bus.members[&agent("claude-1")].task, tasks[1]);
    assert_eq!(bus.members[&agent("glm-1")].task, tasks[3]);
    let state = f.state();
    let crew = state.team(&team).unwrap().crew().roster();
    let roles: Vec<String> = crew
        .iter()
        .map(|(role, m)| format!("{role} {}", m.agent().name()))
        .collect();
    assert_eq!(
        roles,
        [
            "coordinator inv-opus",
            "member claude-1",
            "member claude-2",
            "worker glm-1"
        ]
    );
    f.stop().await.unwrap();
}

#[tokio::test]
async fn the_members_get_the_items_in_order_and_a_member_without_one_waits_with_the_workers() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        normal("codex-1", "sleep"),
        worker("glm-1", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f
        .start_team(&["inv-opus", "claude-1", "claude-2", "codex-1", "glm-1"], PARALLEL)
        .await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }

    let state = f.state();
    let items = state.team(&team).unwrap().items().to_vec();
    assert!(matches!(items[0].phase(), ItemPhase::InProgress { assignee, .. } if assignee.as_str() == "claude-1"));
    assert!(matches!(items[1].phase(), ItemPhase::InProgress { assignee, .. } if assignee.as_str() == "claude-2"));
    let idle = f.prompt(&tasks[3], 1).await;
    assert!(!idle.contains("## Your item"), "the third member has no item: {idle}");
    assert!(idle.contains("Work only on the item"), "and is told to wait: {idle}");
    for member in ["inv-opus", "claude-1", "claude-2", "codex-1", "glm-1"] {
        f.bus_state_is(&team, member, MemberState::Sleeping).await;
    }
    f.stop().await.unwrap();
}

#[tokio::test]
async fn the_first_prompt_of_a_member_has_its_item_and_its_team() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "glm-1"], PARALLEL).await;
    let prompt = f.prompt(&tasks[1], 1).await;
    let coordinator = f.prompt(&tasks[0], 1).await;
    f.stop().await.unwrap();

    assert!(prompt.contains("Your role: member"), "{prompt}");
    assert!(prompt.contains("## Your item: 1.1 — One"), "{prompt}");
    assert!(prompt.contains("## Your team"), "{prompt}");
    assert!(prompt.contains(team.as_str()), "{prompt}");
    assert!(prompt.contains("Coordinator: inv-opus"), "{prompt}");
    assert!(coordinator.contains("Your role: coordinator"), "{coordinator}");
}

#[tokio::test]
async fn an_investigator_in_the_team_coordinates_and_an_explicit_coordinator_wins() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
    ]);
    f.start().await;

    let (first, _) = f.start_team(&["claude-1", "inv-opus"], PARALLEL).await;
    let explicit = f
        .run(&["claude-1", "claude-2"], PARALLEL, Some("claude-2"))
        .await
        .unwrap();
    let Reply::Started { team: second, .. } = explicit else {
        panic!("{explicit:?}")
    };

    let state = f.state();
    let coordinator = |team: &TeamId| {
        state
            .team(team)
            .unwrap()
            .crew()
            .coordinator()
            .unwrap()
            .agent()
            .name()
            .to_owned()
    };
    assert_eq!(coordinator(&first), "inv-opus");
    assert_eq!(coordinator(&second), "claude-2");
    f.stop().await.unwrap();
}

// ---- refusals ----------------------------------------------------------------------------------------------------

/// Nothing was made: no team, no worktree, no branch but `main`.
fn assert_nothing_started(f: &Fixture) {
    assert!(f.state().teams().is_empty(), "a team was made");
    let found: Vec<PathBuf> = std::fs::read_dir(f.home.root().join("worktrees/example"))
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    assert_eq!(found, Vec::<PathBuf>::new(), "a worktree was made");
    assert_eq!(f.branches(), ["main"]);
}

fn all_agents() -> Vec<Agent> {
    vec![
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        worker("glm-1", "sleep"),
    ]
}

#[tokio::test]
async fn a_run_with_too_many_investigators_for_two_outside_reviewers_is_refused_before_anything_starts() {
    let mut f = Fixture::new(&all_agents());
    f.start().await;

    let reply = f.run(&["inv-opus", "inv-codex", "claude-1"], PARALLEL, None).await;

    assert_eq!(refused(reply), FailureKind::Refused);
    assert_nothing_started(&f);
    f.stop().await.unwrap();
}

#[tokio::test]
async fn a_run_with_a_worker_as_coordinator_or_only_workers_is_refused_before_anything_starts() {
    let mut f = Fixture::new(&all_agents());
    f.start().await;

    let as_coordinator = f.run(&["claude-1", "glm-1"], PARALLEL, Some("glm-1")).await;
    let alone = f.run(&["glm-1"], PARALLEL, None).await;

    assert_eq!(refused(as_coordinator), FailureKind::Invalid);
    assert_eq!(refused(alone), FailureKind::Invalid);
    assert_nothing_started(&f);
    f.stop().await.unwrap();
}

#[tokio::test]
async fn a_run_that_names_an_unknown_agent_or_repository_or_has_nothing_to_do_is_refused() {
    let mut f = Fixture::new(&all_agents());
    f.start().await;

    let unknown_agent = f.run(&["claude-1", "nobody"], PARALLEL, None).await;
    let mut elsewhere = f.request(&["claude-1"], PARALLEL, None);
    elsewhere.repo = "elsewhere".parse().unwrap();
    let unknown_repo = f.run_request(elsewhere).await;
    let nothing_to_do = f.run(&["claude-1"], ALL_DONE, None).await;

    assert_eq!(refused(unknown_agent), FailureKind::Invalid);
    assert_eq!(refused(unknown_repo), FailureKind::Invalid);
    assert_eq!(refused(nothing_to_do), FailureKind::Invalid);
    assert_nothing_started(&f);
    f.stop().await.unwrap();
}

#[tokio::test]
async fn a_checkout_without_the_base_branch_is_refused_and_nothing_is_left_behind() {
    let mut f = Fixture::new(&all_agents());
    f.start().await;
    git(&f.repo, &["branch", "-m", "main", "trunk"]);

    let reply = f.run(&["claude-1", "inv-opus"], PARALLEL, None).await;

    assert_eq!(refused(reply), FailureKind::Invalid);
    assert_eq!(f.state().teams(), []);
    assert_eq!(f.branches(), ["trunk"]);
    f.stop().await.unwrap();
}

#[tokio::test]
async fn a_team_the_bus_cannot_create_is_refused_and_its_worktrees_and_branches_are_removed_again() {
    let mut f = Fixture::with_agentbus(&all_agents(), Path::new("/nonexistent/agentbus"));
    f.start().await;

    let reply = f.run(&["claude-1", "inv-opus"], PARALLEL, None).await;

    assert_eq!(refused(reply), FailureKind::Internal);
    assert_nothing_started(&f);
    f.stop().await.unwrap();
}

// ---- the team's life ---------------------------------------------------------------------------------------------

/// The teams on the bus, with whether each is done.
fn bus_teams_done(f: &Fixture) -> Vec<bool> {
    let text = std::fs::read_to_string(f.home.root().join("bus/state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&text).unwrap();
    state["teams"]
        .as_object()
        .map(|teams| {
            teams
                .values()
                .map(|team| team["done"] == serde_json::Value::Bool(true))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_team_whose_state_cannot_be_saved_is_refused_and_what_was_made_for_it_is_removed() {
    use std::os::unix::fs::PermissionsExt as _;

    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
    ]);
    f.start().await;
    // The worktrees and the bus have their directories; the state, saved through a file beside it, has none it can write.
    std::fs::create_dir_all(f.home.root().join("worktrees/example")).unwrap();
    std::fs::create_dir_all(f.home.root().join("bus")).unwrap();
    let root = f.home.root().to_path_buf();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();

    let reply = f.run(&["inv-opus", "claude-1"], PARALLEL, None).await;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    f.answered().await;
    f.stop().await.unwrap();

    assert_eq!(refused(reply), FailureKind::Internal);
    assert_nothing_started(&f);
    let teams = bus_teams_done(&f);
    assert!(!teams.is_empty(), "the team was made on the bus");
    assert!(
        teams.iter().all(|done| *done),
        "the team on the bus is not closed: {teams:?}"
    );
}

#[tokio::test]
async fn a_member_that_is_done_tells_the_coordinator_which_wakes_to_integrate_its_branch() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "slowdone"),
        normal("claude-2", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "claude-2"], PARALLEL).await;
    let woken = f.prompt(&tasks[0], 2).await;
    // The second run of the coordinator is over, and so is the work of the member.
    let state = f
        .state_where("the second run of the coordinator to end", |state| {
            let coordinator = state.find_task(&tasks[0]).unwrap().member;
            coordinator.runs().len() == 2 && coordinator.status() == TaskStatus::Waiting
        })
        .await;
    f.stop().await.unwrap();

    assert!(woken.starts_with("# YOU HAVE BEEN WOKEN UP (team "), "{woken}");
    let messages = f.messages();
    assert!(messages.contains("member_finished"), "{messages}");
    assert!(messages.contains("claude-1 reported DONE on item 1.1"), "{messages}");
    assert!(
        messages.contains(&format!("agents/claude-1/{}-1.1", tasks[1])),
        "{messages}"
    );
    // The item stays with the member until its pull request is open.
    let phase = state.team(&team).unwrap().items()[0].phase().clone();
    assert!(matches!(phase, ItemPhase::InProgress { .. }), "{phase:?}");
    let log_file = f.worktree(&tasks[0]).join(".fake-log");
    let log = std::fs::read_to_string(&log_file).unwrap();
    assert!(!log.contains("git failed"), "{log}");
    let sessions: Vec<&str> = log.lines().map(|line| line.split(' ').nth(1).unwrap()).collect();
    assert_eq!(
        sessions[0], sessions[1],
        "the coordinator is resumed, not started again: {log}"
    );
}

#[tokio::test]
async fn a_member_that_failed_tells_the_coordinator_so() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "slowfail"),
    ]);
    f.start().await;

    let (_, tasks) = f.start_team(&["inv-opus", "claude-1"], PARALLEL).await;
    f.status_is(&tasks[1], TaskStatus::Failed).await;
    f.prompt(&tasks[0], 2).await;
    f.stop().await.unwrap();

    let messages = f.messages();
    assert!(messages.contains("member_failed"), "{messages}");
    assert!(messages.contains("claude-1 failed"), "{messages}");
}

#[tokio::test]
async fn a_message_to_a_member_that_sleeps_starts_its_task_again_with_the_wake() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1"], PLAIN).await;
    f.status_is(&tasks[0], TaskStatus::Waiting).await;
    assert_eq!(f.runs(&tasks[1]), 1, "its first run slept: {}", f.messages());

    f.bus()
        .send(
            &agent("inv-opus"),
            &Recipient::Agent(agent("claude-1")),
            &Outgoing::Chat {
                text: "Please read the README.".to_owned(),
            },
        )
        .await
        .unwrap();
    let prompt = f.prompt(&tasks[1], 2).await;
    f.status_is(&tasks[1], TaskStatus::Waiting).await;
    f.stop().await.unwrap();

    assert!(prompt.starts_with("# YOU HAVE BEEN WOKEN UP (team "), "{prompt}");
    assert!(prompt.contains(team.as_str()), "{prompt}");
    assert_eq!(f.runs(&tasks[0]), 1, "the coordinator was not woken: {}", f.messages());
}

#[tokio::test]
async fn a_request_for_a_worker_goes_to_the_first_free_worker_and_its_result_returns_to_the_requester() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "sleep,done:Result: three files."),
        worker("glm-2", "sleep,done:Result: other."),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "glm-1", "glm-2"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    assert_eq!(f.runs(&tasks[2]), 1, "a worker is not started again before it is asked");

    let id = f
        .bus()
        .delegate(&Delegation {
            team: team.clone(),
            requester: agent("claude-1"),
            task: tasks[1].clone(),
            repo: "example".parse().unwrap(),
            branch: format!("agents/claude-1/{}", tasks[1]).parse().unwrap(),
            worktree: abs(&f.worktree(&tasks[1])),
            dispatcher: Program::from_str("true").unwrap(),
            request: "List the files under src/.".to_owned(),
            context: "The module is small.".to_owned(),
        })
        .await
        .unwrap();
    let prompt = f.prompt(&tasks[2], 2).await;
    let request = f.request_with(id, DelegationStatus::Done).await;
    f.stop().await.unwrap();

    assert!(prompt.contains("A REQUEST FROM claude-1 (H1)"), "{prompt}");
    assert!(prompt.contains("List the files under src/."), "{prompt}");
    assert_eq!(
        request.worker,
        Some(agent("glm-1")),
        "the first free worker in the configured order"
    );
    assert_eq!(request.helper_task, Some(tasks[2].clone()));
    assert!(request.summary.contains("Result: three files."), "{}", request.summary);
    assert!(
        !request.summary.contains("STATUS:"),
        "the status line is not the result: {}",
        request.summary
    );
    assert_eq!(f.runs(&tasks[3]), 1, "the second worker was not needed");
    let messages = f.messages();
    assert!(messages.contains("helper_result"), "{messages}");
}

#[tokio::test]
async fn a_worker_that_fails_returns_the_failure_to_the_requester() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "sleep,fail"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "glm-1"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }

    let id = f
        .bus()
        .delegate(&Delegation {
            team,
            requester: agent("claude-1"),
            task: tasks[1].clone(),
            repo: "example".parse().unwrap(),
            branch: format!("agents/claude-1/{}", tasks[1]).parse().unwrap(),
            worktree: abs(&f.worktree(&tasks[1])),
            dispatcher: Program::from_str("true").unwrap(),
            request: "Do it.".to_owned(),
            context: "None.".to_owned(),
        })
        .await
        .unwrap();
    f.status_is(&tasks[2], TaskStatus::Failed).await;
    let request = f.request_with(id, DelegationStatus::Failed).await;
    f.stop().await.unwrap();

    assert!(request.summary.contains("I could not do it."), "{}", request.summary);
    assert!(f.messages().contains("\"status\":\"failed\""), "{}", f.messages());
}

/// A team of two normal agents and a worker, with its ID and tasks; the first agent asks for the worker.
struct Pair {
    team: TeamId,
    tasks: Vec<TaskId>,
    requester: &'static str,
    other: &'static str,
    worker: &'static str,
}

impl Fixture {
    /// Starts the team of `requester`, `other` and `worker` on a list without items.
    async fn start_pair(&self, requester: &'static str, other: &'static str, worker: &'static str) -> Pair {
        let (team, tasks) = self.start_team(&[requester, other, worker], PLAIN).await;
        Pair {
            team,
            tasks,
            requester,
            other,
            worker,
        }
    }

    /// Puts a request of the requester of `pair` for a worker on the bus.
    async fn delegate(&self, pair: &Pair, request: &str) -> RequestId {
        let task = &pair.tasks[0];
        self.bus()
            .delegate(&Delegation {
                team: pair.team.clone(),
                requester: agent(pair.requester),
                task: task.clone(),
                repo: "example".parse().unwrap(),
                branch: format!("agents/{}/{task}", pair.requester).parse().unwrap(),
                worktree: abs(&self.worktree(task)),
                dispatcher: Program::from_str("true").unwrap(),
                request: request.to_owned(),
                context: "None.".to_owned(),
            })
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn the_requests_of_two_teams_on_one_bus_are_each_served_by_a_worker_of_their_own_team() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        worker("glm-1", "sleep,done:Result: one."),
        normal("claude-3", "sleep"),
        normal("claude-4", "sleep"),
        worker("glm-2", "sleep,done:Result: two."),
    ]);
    f.start().await;
    let one = f.start_pair("claude-1", "claude-2", "glm-1").await;
    let two = f.start_pair("claude-3", "claude-4", "glm-2").await;
    // The daemon looks at the teams in the order of their IDs: the oldest request on the bus is the later team's, so
    // the earlier team, which has a free worker, sees it first.
    let (earlier, later) = if one.team < two.team { (one, two) } else { (two, one) };
    // A team the daemon looked at after its last run sleeps on the bus; from then on both teams are looked at together.
    for pair in [&earlier, &later] {
        for (task, member) in pair.tasks.iter().zip([pair.requester, pair.other, pair.worker]) {
            f.status_is(task, TaskStatus::Waiting).await;
            f.bus_state_is(&pair.team, member, MemberState::Sleeping).await;
        }
    }

    let of_later = f.delegate(&later, "Count the lines.").await;
    let of_earlier = f.delegate(&earlier, "List the files.").await;
    let later_served = f.request_with(of_later, DelegationStatus::Done).await;
    let earlier_served = f.request_with(of_earlier, DelegationStatus::Done).await;
    f.stop().await.unwrap();

    assert_eq!(later_served.worker, Some(agent(later.worker)));
    assert_eq!(later_served.helper_task, Some(later.tasks[2].clone()));
    assert_eq!(earlier_served.worker, Some(agent(earlier.worker)));
    assert_eq!(earlier_served.helper_task, Some(earlier.tasks[2].clone()));
}

#[tokio::test]
async fn a_worker_whose_run_cannot_start_fails_the_request_and_the_requester_is_told() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        worker("glm-1", "sleep,done:Result: never."),
    ]);
    f.start().await;
    let pair = f.start_pair("claude-1", "claude-2", "glm-1").await;
    for task in &pair.tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    std::fs::remove_dir_all(f.worktree(&pair.tasks[2])).unwrap();

    let id = f.delegate(&pair, "Do it.").await;
    let request = f.request_with(id, DelegationStatus::Failed).await;
    let state = f.status_is(&pair.tasks[2], TaskStatus::Failed).await;
    f.wait_for("the requester to be told", || {
        f.messages().contains("\"status\":\"failed\"")
    })
    .await;
    f.stop().await.unwrap();

    assert_eq!(request.worker, Some(agent("glm-1")));
    assert_eq!(
        state.find_task(&pair.tasks[2]).unwrap().member.runs().len(),
        1,
        "the worker had no second run"
    );
}

#[tokio::test]
async fn a_member_whose_run_cannot_start_tells_the_coordinator() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
    ]);
    f.start().await;
    let (_, tasks) = f.start_team(&["inv-opus", "claude-1"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    std::fs::remove_dir_all(f.worktree(&tasks[1])).unwrap();

    f.bus()
        .send(
            &agent("inv-opus"),
            &Recipient::Agent(agent("claude-1")),
            &Outgoing::Chat {
                text: "Please read the README.".to_owned(),
            },
        )
        .await
        .unwrap();
    f.status_is(&tasks[1], TaskStatus::Failed).await;
    f.wait_for("the coordinator to be told", || {
        f.messages().contains("member_no_status")
    })
    .await;
    f.prompt(&tasks[0], 2).await;
    f.stop().await.unwrap();

    assert!(f.messages().contains("claude-1 ended its turn"), "{}", f.messages());
}

#[tokio::test]
async fn a_worker_whose_questions_cannot_be_used_fails_the_request_as_one_without_a_result() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        worker("glm-1", "sleep,questions"),
    ]);
    f.start().await;
    let pair = f.start_pair("claude-1", "claude-2", "glm-1").await;
    for task in &pair.tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }

    let id = f.delegate(&pair, "Do it.").await;
    let request = f.request_with(id, DelegationStatus::Failed).await;
    f.stop().await.unwrap();

    let sentence = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/harness/team/worker-no-result.md"),
    )
    .unwrap();
    assert!(request.summary.starts_with(sentence.trim_end()), "{}", request.summary);
}

#[tokio::test]
async fn a_team_whose_lead_is_done_and_whose_members_wait_is_closed_and_the_human_is_told_once() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "slowdone"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "glm-1"], PLAIN).await;
    let state = f.status_is(&tasks[0], TaskStatus::Done).await;
    f.wait_for("the notification", || {
        f.notifications().iter().any(|text| text.contains("is finished"))
    })
    .await;
    let bus = loop {
        let bus = f.bus().team_get(&team).await.unwrap();
        if bus.done {
            break bus;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    f.stop().await.unwrap();

    assert_eq!(task_status(&state, &tasks[1]), TaskStatus::Done, "a member that waited");
    assert_eq!(task_status(&state, &tasks[2]), TaskStatus::Done, "a worker that waited");
    assert!(bus.done);
    let notices: Vec<String> = f
        .notifications()
        .into_iter()
        .filter(|text| text.contains("is finished"))
        .collect();
    assert_eq!(notices.len(), 1, "the human is told once: {notices:?}");
}

#[tokio::test]
async fn a_closed_team_is_not_closed_again_when_the_daemon_restarts() {
    let mut f = Fixture::new(&[
        normal("claude-1", "done"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
    ]);
    f.start().await;
    let (_, tasks) = f.start_team(&["claude-1"], PLAIN).await;
    f.status_is(&tasks[0], TaskStatus::Done).await;
    f.wait_for("the notification", || {
        f.notifications().iter().any(|text| text.contains("is finished"))
    })
    .await;
    f.stop().await.unwrap();

    f.start().await;
    f.answered().await;
    f.stop().await.unwrap();

    let notices = f
        .notifications()
        .into_iter()
        .filter(|text| text.contains("is finished"))
        .count();
    assert_eq!(notices, 1);
}

#[tokio::test]
async fn a_member_whose_repository_left_the_configuration_fails_at_its_next_start_and_the_coordinator_is_told() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "slowdone"),
    ]);
    f.start().await;
    let (_, tasks) = f.start_team(&["inv-opus", "claude-1"], PARALLEL).await;
    f.status_is(&tasks[1], TaskStatus::Running).await;
    f.stop().await.unwrap();
    assert_eq!(
        task_status(&f.state(), &tasks[1]),
        TaskStatus::Queued,
        "the run was interrupted"
    );
    let config = std::fs::read_to_string(f.home.config_file()).unwrap();
    let without = config.replace("[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n", "");
    assert_ne!(without, config, "the repository is in the configuration");
    std::fs::write(f.home.config_file(), without).unwrap();

    f.start().await;
    f.status_is(&tasks[1], TaskStatus::Failed).await;
    f.wait_for("the coordinator to be told", || {
        f.messages().contains("member_no_status")
    })
    .await;
    f.stop().await.unwrap();

    assert!(f.messages().contains("claude-1 ended its turn"), "{}", f.messages());
}

#[tokio::test]
async fn a_free_member_is_given_the_next_item_when_the_daemon_looks_at_the_team_again() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "done"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1"], PARALLEL).await;
    f.status_is(&tasks[1], TaskStatus::Waiting).await;
    f.status_is(&tasks[0], TaskStatus::Waiting).await;
    f.stop().await.unwrap();
    // The first item has its pull request open (a later item of the plan does that): the member is free.
    let mut state = f.state();
    {
        let mut team_mut = state.team_mut(&team).unwrap();
        let mut item = team_mut.item_mut("1.1".parse().unwrap()).unwrap();
        let branch = format!("agents/claude-1/{}-1.1", tasks[1]).parse().unwrap();
        item.advance(ItemPhase::InReview {
            pr: PullRequest {
                number: PrNumber::new(std::num::NonZeroU32::new(5).unwrap()),
                branch,
                opened_at: Timestamp::now(&agent_runtime::clock::SystemClock).unwrap(),
            },
            reviews: Vec::new(),
        })
        .unwrap();
    }
    std::fs::write(f.home.state_file(), state.to_json().unwrap()).unwrap();

    f.start().await;
    let prompt = f.prompt(&tasks[1], 2).await;
    let state = f.status_is(&tasks[1], TaskStatus::Waiting).await;
    f.stop().await.unwrap();

    assert!(prompt.starts_with("# A NEW ITEM FOR YOU: 2.1"), "{prompt}");
    assert!(prompt.contains("## Your item: 2.1 — Two"), "{prompt}");
    let branch = format!("agents/claude-1/{}-2.1", tasks[1]);
    assert_eq!(
        git(&f.worktree(&tasks[1]), &["rev-parse", "--abbrev-ref", "HEAD"]),
        branch
    );
    let phase = state.team(&team).unwrap().items()[1].phase().clone();
    assert!(
        matches!(&phase, ItemPhase::InProgress { assignee, branch: b } if assignee.as_str() == "claude-1" && b.as_str() == branch),
        "{phase:?}"
    );
}

#[tokio::test]
async fn a_bus_that_is_down_is_tried_again_and_nothing_is_lost() {
    let flaky = fixture_script("flaky-agentbus.sh");
    let mut f = Fixture::with_agentbus(
        &[
            investigator("inv-opus", "sleep"),
            investigator("inv-codex", "sleep"),
            investigator("inv-gemini", "sleep"),
            normal("claude-1", "slowdone"),
        ],
        &flaky,
    );
    f.start().await;
    let (_, tasks) = f.start_team(&["inv-opus", "claude-1"], PARALLEL).await;
    f.status_is(&tasks[0], TaskStatus::Waiting).await;
    let down = f.home.root().join("flaky-bus-down");
    std::fs::write(&down, "").unwrap();

    f.status_is(&tasks[1], TaskStatus::Waiting).await;
    let refused = f.home.root().join("flaky-bus-refused");
    f.wait_for("the notice for the coordinator to be refused", || {
        std::fs::read_to_string(&refused).is_ok_and(|calls| calls.lines().any(|call| call == "send"))
    })
    .await;
    assert_eq!(
        f.runs(&tasks[0]),
        1,
        "the coordinator cannot be told while the bus is down"
    );
    std::fs::remove_file(&down).unwrap();

    let woken = f.prompt(&tasks[0], 2).await;
    f.stop().await.unwrap();

    assert!(woken.contains("YOU HAVE BEEN WOKEN UP"), "{woken}");
    assert!(
        f.messages().contains("claude-1 reported DONE on item 1.1"),
        "{}",
        f.messages()
    );
}

// ---- what the bus refuses, and what git refuses ------------------------------------------------------------------

mod refusals {
    use agent_runtime::agentbus::{AgentBusError, BusCommand, FailureClass};
    use agent_runtime::process::CaptureError;

    use super::super::is_transient;

    fn failed(class: FailureClass, exit_code: i32) -> AgentBusError {
        AgentBusError::Failed {
            command: BusCommand::HelperSet,
            exit_code,
            class,
            message: "refused".to_owned(),
        }
    }

    #[test]
    fn a_bus_that_cannot_be_reached_or_has_trouble_with_its_files_is_tried_again() {
        for error in [
            failed(FailureClass::Storage, 8),
            failed(FailureClass::Process, 10),
            failed(FailureClass::Unclassified, 1),
            AgentBusError::Run {
                command: BusCommand::Send,
                source: CaptureError::Timeout {
                    limit: std::time::Duration::from_secs(1),
                },
            },
        ] {
            assert!(is_transient(&error), "{error:?}");
        }
    }

    #[test]
    fn a_request_the_bus_refuses_for_good_is_not_tried_again() {
        for (class, code) in [
            (FailureClass::InvalidArgument, 2),
            (FailureClass::NotFound, 3),
            (FailureClass::NotInTeam, 4),
            (FailureClass::Permission, 5),
            (FailureClass::Conflict, 6),
            (FailureClass::InvalidState, 7),
            (FailureClass::UnknownTool, 9),
        ] {
            assert!(!is_transient(&failed(class, code)), "{class:?}");
        }
    }
}

#[tokio::test]
async fn what_the_bus_refuses_for_good_is_dropped_and_what_follows_is_done() {
    let flaky = fixture_script("flaky-agentbus.sh");
    let mut f = Fixture::with_agentbus(
        &[
            investigator("inv-opus", "sleep"),
            investigator("inv-codex", "sleep"),
            investigator("inv-gemini", "sleep"),
            normal("claude-1", "slowdone"),
            normal("claude-2", "done"),
        ],
        &flaky,
    );
    f.start().await;
    let refuse = f.home.root().join("flaky-bus-refuse-once");
    std::fs::write(&refuse, "send").unwrap();

    let (_, tasks) = f.start_team(&["inv-opus", "claude-1", "claude-2"], PARALLEL).await;
    f.wait_for("the notice about claude-2 to be refused", || !refuse.exists())
        .await;
    let woken = f.prompt(&tasks[0], 2).await;
    f.stop().await.unwrap();

    assert!(woken.contains("YOU HAVE BEEN WOKEN UP"), "{woken}");
    let messages = f.messages();
    assert!(messages.contains("claude-1 reported DONE on item 1.1"), "{messages}");
    assert!(
        !messages.contains("claude-2 reported DONE"),
        "the refused notice is not sent again: {messages}"
    );
}

#[tokio::test]
async fn an_item_whose_branch_cannot_be_started_waits_and_the_daemon_goes_on() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "done"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1"], PARALLEL).await;
    f.status_is(&tasks[1], TaskStatus::Waiting).await;
    f.status_is(&tasks[0], TaskStatus::Waiting).await;
    f.stop().await.unwrap();
    // The branch of the next item exists already, so git cannot start it again.
    git(
        &f.repo,
        &["branch", &format!("agents/claude-1/{}-2.1", tasks[1]), "main"],
    );
    let mut state = f.state();
    {
        let mut team_mut = state.team_mut(&team).unwrap();
        let mut item = team_mut.item_mut("1.1".parse().unwrap()).unwrap();
        item.advance(ItemPhase::InReview {
            pr: PullRequest {
                number: PrNumber::new(std::num::NonZeroU32::new(5).unwrap()),
                branch: format!("agents/claude-1/{}-1.1", tasks[1]).parse().unwrap(),
                opened_at: Timestamp::now(&agent_runtime::clock::SystemClock).unwrap(),
            },
            reviews: Vec::new(),
        })
        .unwrap();
    }
    std::fs::write(f.home.state_file(), state.to_json().unwrap()).unwrap();

    f.start().await;
    f.answered().await;
    let state = f.state();
    f.stop().await.unwrap();

    let phase = state.team(&team).unwrap().items()[1].phase().clone();
    assert!(matches!(phase, ItemPhase::Pending), "{phase:?}");
    assert_eq!(task_status(&state, &tasks[1]), TaskStatus::Waiting);
    assert_eq!(
        f.runs(&tasks[1]),
        1,
        "the member was not woken for an item it was not given"
    );
}

#[tokio::test]
async fn an_item_whose_branch_could_not_be_started_is_given_out_when_the_branch_can_be_made() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "done"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1"], PARALLEL).await;
    f.status_is(&tasks[1], TaskStatus::Waiting).await;
    // The coordinator is woken for the member's DONE; when that run is over, nothing is left for the team to do.
    f.prompt(&tasks[0], 2).await;
    f.state_where("the second run of the coordinator to end", |state| {
        let coordinator = state.find_task(&tasks[0]).unwrap().member;
        coordinator.runs().len() == 2 && coordinator.status() == TaskStatus::Waiting
    })
    .await;
    f.stop().await.unwrap();
    // The branch of the next item exists already, so git cannot start it until it is removed.
    let next_branch = format!("agents/claude-1/{}-2.1", tasks[1]);
    git(&f.repo, &["branch", &next_branch, "main"]);
    let mut state = f.state();
    {
        let mut team_mut = state.team_mut(&team).unwrap();
        let mut item = team_mut.item_mut("1.1".parse().unwrap()).unwrap();
        item.advance(ItemPhase::InReview {
            pr: PullRequest {
                number: PrNumber::new(std::num::NonZeroU32::new(5).unwrap()),
                branch: format!("agents/claude-1/{}-1.1", tasks[1]).parse().unwrap(),
                opened_at: Timestamp::now(&agent_runtime::clock::SystemClock).unwrap(),
            },
            reviews: Vec::new(),
        })
        .unwrap();
    }
    std::fs::write(f.home.state_file(), state.to_json().unwrap()).unwrap();
    f.start().await;
    // The daemon's first look after the start could not give the item out, and no run is left to end: only the
    // daemon's clock can give the item out from now on.
    f.answered().await;
    assert_eq!(f.runs(&tasks[1]), 1, "the item cannot start yet");
    assert_eq!(f.runs(&tasks[0]), 2, "the coordinator has nothing to do");

    git(&f.repo, &["branch", "-D", &next_branch]);
    let prompt = f.prompt(&tasks[1], 2).await;
    f.stop().await.unwrap();

    assert!(prompt.starts_with("# A NEW ITEM FOR YOU: 2.1"), "{prompt}");
    assert_eq!(
        git(&f.worktree(&tasks[1]), &["rev-parse", "--abbrev-ref", "HEAD"]),
        next_branch
    );
}

// ---- the rounds of a team ----------------------------------------------------------------------------------------

/// The rounds the run number `n` (from 0) of `task` works for, as the state says.
fn rounds_of(state: &State, task: &TaskId, n: usize) -> Vec<agent_core::ids::RoundId> {
    state.find_task(task).unwrap().member.runs()[n].rounds().to_vec()
}

/// The ends of the rounds of the only team of `state`, oldest round first.
fn ends_of(state: &State) -> Vec<Option<agent_core::state::RoundEnd>> {
    state.teams()[0]
        .rounds()
        .iter()
        .map(agent_core::state::Round::ended)
        .collect()
}

#[tokio::test]
async fn the_first_items_of_a_team_get_a_round_each_when_the_team_starts_and_every_run_names_the_rounds_of_its_role() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "claude-2"], PARALLEL).await;
    let state = f
        .state_where("every task to have run once", |state| {
            tasks
                .iter()
                .all(|task| !state.find_task(task).unwrap().member.runs().is_empty())
        })
        .await;
    f.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    let first = found.round_of("1.1".parse().unwrap()).unwrap().id().clone();
    let second = found.round_of("2.1".parse().unwrap()).unwrap().id().clone();
    assert_eq!(
        found
            .rounds()
            .iter()
            .map(agent_core::state::Round::id)
            .collect::<Vec<_>>(),
        [&first, &second]
    );
    assert!(found.rounds().iter().all(agent_core::state::Round::is_open));
    assert_eq!(
        rounds_of(&state, &tasks[1], 0),
        std::slice::from_ref(&first),
        "a member: the round of its item"
    );
    assert_eq!(
        rounds_of(&state, &tasks[2], 0),
        std::slice::from_ref(&second),
        "a member: the round of its item"
    );
    assert_eq!(
        rounds_of(&state, &tasks[0], 0),
        [first, second],
        "the coordinator: every round that is open"
    );
}

#[tokio::test]
async fn a_list_without_items_is_one_round_that_its_solo_agent_works_for_and_that_ends_with_the_team() {
    let mut f = Fixture::new(&[
        normal("claude-1", "done"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
    ]);
    f.start().await;

    let (team, tasks) = f.start_team(&["claude-1"], PLAIN).await;
    f.status_is(&tasks[0], TaskStatus::Done).await;
    f.wait_for("the notification that the team is finished", || {
        f.notifications().iter().any(|text| text.contains("is finished"))
    })
    .await;
    let state = f
        .state_where("the round to be closed", |state| {
            ends_of(state) == [Some(agent_core::state::RoundEnd::Abandoned)]
        })
        .await;
    f.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    assert_eq!(found.rounds().len(), 1);
    let round = &found.rounds()[0];
    assert_eq!(round.item(), None);
    assert_eq!(rounds_of(&state, &tasks[0], 0), [round.id().clone()]);
    assert_eq!(task_status(&state, &tasks[0]), TaskStatus::Done);
}

#[tokio::test]
async fn in_a_team_on_a_list_without_items_the_coordinator_works_for_the_round_and_a_worker_asked_by_a_member_does_not()
{
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "sleep,done:Result: one,done:Result: two"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "glm-1"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    let asks = |requester: &str, task: &TaskId| Delegation {
        team: team.clone(),
        requester: agent(requester),
        task: task.clone(),
        repo: "example".parse().unwrap(),
        branch: format!("agents/{requester}/{task}").parse().unwrap(),
        worktree: abs(&f.worktree(task)),
        dispatcher: Program::from_str("true").unwrap(),
        request: "Look.".to_owned(),
        context: "None.".to_owned(),
    };

    let by_member = f.bus().delegate(&asks("claude-1", &tasks[1])).await.unwrap();
    f.request_with(by_member, DelegationStatus::Done).await;
    let by_coordinator = f.bus().delegate(&asks("inv-opus", &tasks[0])).await.unwrap();
    f.request_with(by_coordinator, DelegationStatus::Done).await;
    let state = f
        .state_where("the worker to have run twice", |state| {
            state.find_task(&tasks[2]).unwrap().member.runs().len() >= 3
        })
        .await;
    f.stop().await.unwrap();

    let round = state.team(&team).unwrap().rounds()[0].id().clone();
    assert_eq!(
        rounds_of(&state, &tasks[0], 0),
        std::slice::from_ref(&round),
        "the coordinator"
    );
    assert_eq!(rounds_of(&state, &tasks[1], 0), [], "a member without an item");
    // The first run of the worker was its start; then it served the member's request and the coordinator's.
    assert_eq!(rounds_of(&state, &tasks[2], 1), [], "asked by a run without rounds");
    assert_eq!(rounds_of(&state, &tasks[2], 2), [round], "asked by a run with a round");
}

#[tokio::test]
async fn a_stop_that_leaves_the_team_nobody_to_work_abandons_its_open_rounds_and_a_stop_of_one_task_does_not() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1", "claude-2"], PARALLEL).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }

    let stop = |target: Target| Command::Stop(agent_core::protocol::ControlRequest { target });
    f.connect()
        .await
        .request(Client::Cli, stop(Target::Task(tasks[1].clone())))
        .await
        .unwrap();
    let one_task = f
        .state_where("the task to be stopped", |state| {
            task_status(state, &tasks[1]) == TaskStatus::Stopped
        })
        .await;
    f.connect()
        .await
        .request(Client::Cli, stop(Target::Team(team.clone())))
        .await
        .unwrap();
    let whole = f
        .state_where("the rounds to be abandoned", |state| {
            ends_of(state) == [Some(agent_core::state::RoundEnd::Abandoned); 2]
        })
        .await;
    f.stop().await.unwrap();

    assert_eq!(ends_of(&one_task), [None, None], "one task stopped, the others wait");
    assert!(whole.teams()[0].rounds().iter().all(|round| !round.is_open()));
    assert!(whole.teams()[0].open_rounds().next().is_none());
}

#[tokio::test]
async fn two_teams_started_at_once_never_share_a_round() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        normal("claude-2", "sleep"),
        normal("claude-3", "sleep"),
        normal("claude-4", "sleep"),
    ]);
    f.start().await;

    let (first, first_tasks) = f.start_team(&["inv-opus", "claude-1", "claude-2"], PARALLEL).await;
    let (second, second_tasks) = f.start_team(&["inv-codex", "claude-3", "claude-4"], PARALLEL).await;
    let state = f
        .state_where("every task to have run once", |state| {
            first_tasks
                .iter()
                .chain(&second_tasks)
                .all(|task| !state.find_task(task).unwrap().member.runs().is_empty())
        })
        .await;
    f.stop().await.unwrap();

    let ids = |team: &TeamId| -> Vec<agent_core::ids::RoundId> {
        state
            .team(team)
            .unwrap()
            .rounds()
            .iter()
            .map(|round| round.id().clone())
            .collect()
    };
    let (mine, theirs) = (ids(&first), ids(&second));
    assert_eq!((mine.len(), theirs.len()), (2, 2));
    assert!(mine.iter().all(|round| !theirs.contains(round)), "{mine:?} {theirs:?}");
    for (tasks, own) in [(&first_tasks, &mine), (&second_tasks, &theirs)] {
        for task in tasks {
            assert!(
                rounds_of(&state, task, 0).iter().all(|round| own.contains(round)),
                "{task}"
            );
        }
    }
}

#[tokio::test]
async fn a_daemon_stopped_in_the_middle_of_a_round_goes_on_with_the_same_round() {
    let mut f = Fixture::new(&[
        normal("claude-1", "done"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["claude-1"], CHAIN).await;
    let before = f.status_is(&tasks[0], TaskStatus::Waiting).await;
    f.stop().await.unwrap();

    f.start().await;
    f.answered().await;
    f.answered().await;
    let after = f.state();
    f.stop().await.unwrap();

    let rounds = |state: &State| -> Vec<(agent_core::ids::RoundId, Option<agent_core::ids::ItemId>, bool)> {
        state
            .team(&team)
            .unwrap()
            .rounds()
            .iter()
            .map(|round| (round.id().clone(), round.item(), round.is_open()))
            .collect()
    };
    assert_eq!(rounds(&before).len(), 1, "the second item waits for the first");
    assert_eq!(rounds(&after), rounds(&before));
    assert_eq!(rounds_of(&after, &tasks[0], 0), [rounds(&before)[0].0.clone()]);
}

#[tokio::test]
async fn a_round_is_found_by_its_id_in_status_and_show_an_unknown_one_is_not_found_and_a_round_is_not_controlled() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
    ]);
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1"], CHAIN).await;
    let state = f
        .state_where("the member to have run", |state| {
            !state.find_task(&tasks[1]).unwrap().member.runs().is_empty()
        })
        .await;
    let round = state.team(&team).unwrap().rounds()[0].id().clone();
    let unknown: agent_core::ids::RoundId = "round-20201111-153001-q1w2".parse().unwrap();
    let ask = |command: Command| async { f.connect().await.request(Client::Cli, command).await };

    let status = ask(Command::Status(StatusRequest {
        target: Some(Target::Round(round.clone())),
    }))
    .await
    .unwrap();
    let shown = ask(Command::Show(agent_core::protocol::ShowRequest {
        target: Target::Round(round.clone()),
    }))
    .await
    .unwrap();
    let missing = ask(Command::Show(agent_core::protocol::ShowRequest {
        target: Target::Round(unknown.clone()),
    }))
    .await;
    let missing_status = ask(Command::Status(StatusRequest {
        target: Some(Target::Round(unknown.clone())),
    }))
    .await;
    let stopped = ask(Command::Stop(agent_core::protocol::ControlRequest {
        target: Target::Round(round.clone()),
    }))
    .await;
    f.stop().await.unwrap();

    let Reply::Teams { teams } = status else {
        panic!("not teams: {status:?}")
    };
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].id, team);
    assert_eq!(teams[0].rounds.len(), 1);
    let Reply::Round { round: detail } = shown else {
        panic!("not a round: {shown:?}")
    };
    assert_eq!((detail.team.clone(), detail.round.id().clone()), (team, round));
    assert_eq!(
        detail
            .item
            .as_ref()
            .map(|item| (item.id.to_string(), item.title.as_str())),
        Some(("1.1".to_owned(), "One"))
    );
    // The member ran for the round; the coordinator did too, but only a task that has a run naming it is listed.
    let listed: Vec<&TaskId> = detail.tasks.iter().map(|task| &task.task).collect();
    assert!(listed.contains(&&tasks[1]), "{listed:?}");
    let member = detail.tasks.iter().find(|task| task.task == tasks[1]).unwrap();
    assert_eq!(member.worktree.as_path(), f.worktree(&tasks[1]));
    assert_eq!(member.branch.as_str(), format!("agents/claude-1/{}-1.1", tasks[1]));
    assert_eq!(refused(missing), FailureKind::NotFound);
    assert_eq!(refused(missing_status), FailureKind::NotFound);
    assert_eq!(refused(stopped), FailureKind::Invalid);
}
