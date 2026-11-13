//! A daemon in a temporary home for the tests of the autopilot: a scratch repository with a bare `origin`, a fake
//! GitHub, a stand-in for the model CLI that does what each agent's scenario says, and the real `agentbus` when a team
//! needs one.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use agent_core::config::AbsolutePath;
use agent_core::ids::{AgentName, Clock, TaskId, TeamId};
use agent_core::protocol::{AgentList, Client, Command, Reply, RunRequest};
use agent_core::state::{ItemPhase, MergeMode, State, TaskListText};
use agent_core::status::TaskStatus;
use agent_runtime::clock::SharedClock;
use agent_runtime::run::RunTimings;
use agent_runtime::store::Home;
use time::UtcDateTime;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::fake_github::{ACCOUNT, FakeGitHub};
use crate::client::Connection;
use crate::daemon::{DaemonError, DaemonSetup, Timings, run};

/// The longest any wait of these tests lasts.
pub(super) const PATIENCE: Duration = Duration::from_secs(30);

/// Two features that do not wait for each other would start both items at once; this chain starts the second item when
/// the first is merged.
pub(super) const CHAIN: &str = "\
Build the thing.

1. First
[ ] 1.1 The first item
    Do the first thing in the repository.
    Expected result:
    - the first thing works
    Checks:
    - run the first thing
[ ] 1.2 The second item
    Do the second thing in the repository.
    Expected result:
    - the second thing works
";

/// A list with one item.
pub(super) const ONE: &str = "\
Build the thing.

1. First
[ ] 1.1 The first item
    Do the first thing in the repository.
    Expected result:
    - the first thing works
    Checks:
    - run the first thing
";

/// A clock whose every reading is a second later than the one before: the events of a test have an order that whole
/// seconds of the system clock would blur.
#[derive(Debug, Clone)]
pub(super) struct TickingClock {
    /// The first reading, in seconds since the epoch.
    start: i64,
    /// How many readings there were.
    ticks: Arc<AtomicU32>,
}

impl TickingClock {
    /// A clock that starts now.
    pub(super) fn new() -> Self {
        Self {
            start: UtcDateTime::now().unix_timestamp(),
            ticks: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Clock for TickingClock {
    fn now(&self) -> UtcDateTime {
        let tick = self.ticks.fetch_add(1, Ordering::SeqCst);
        UtcDateTime::from_unix_timestamp(self.start + i64::from(tick)).unwrap()
    }
}

/// An agent of the configuration and what its stand-in CLI does, run after run (see `team-cli.sh`).
#[derive(Debug, Clone)]
pub(super) struct Agent {
    /// The name.
    pub(super) name: &'static str,
    /// Whether it is an investigator.
    pub(super) investigator: bool,
    /// The actions of its runs, comma separated.
    pub(super) scenario: &'static str,
}

/// A normal agent.
pub(super) fn normal(name: &'static str, scenario: &'static str) -> Agent {
    Agent {
        name,
        investigator: false,
        scenario,
    }
}

/// An investigator.
pub(super) fn investigator(name: &'static str, scenario: &'static str) -> Agent {
    Agent {
        name,
        investigator: true,
        scenario,
    }
}

/// Runs git for the setup and the checks of a test and returns its trimmed output.
pub(super) fn git(dir: &Path, args: &[&str]) -> String {
    let output = Process::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "protocol.file.allow=always"])
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

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

/// A fixture script of the `agent` crate.
fn fixture_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// A daemon in a temporary home.
pub(super) struct Rig {
    _dir: tempfile::TempDir,
    pub(super) home: Home,
    /// The checkout the teams are started from, with a bare `origin`.
    pub(super) repo: PathBuf,
    /// The bare repository that stands for GitHub's.
    pub(super) origin: PathBuf,
    pub(super) github: FakeGitHub,
    shutdown: CancellationToken,
    daemon: Option<JoinHandle<Result<(), DaemonError>>>,
    clock: TickingClock,
}

impl Rig {
    /// A home with these agents, the repository `example`, a fake GitHub and the token of its account; the daemon is
    /// not started. `fix_passes` is `[review] fix_passes`; `bus` is whether the agents get the real `agentbus`.
    pub(super) async fn new(agents: &[Agent], fix_passes: u32, bus: bool) -> Self {
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
        let origin = root.join("octo").join("example.git");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let repo = root.join("example");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "Hello\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-q", "-m", "First"]);
        git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&repo, &["push", "-q", "origin", "main"]);
        let github = FakeGitHub::start(&origin).await.unwrap();
        std::fs::write(
            home.config_file(),
            config(agents, &github.url(), fix_passes, &home, bus),
        )
        .unwrap();
        let secrets = home.root().join("secrets.toml");
        std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
        std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        Self {
            _dir: dir,
            home,
            repo,
            origin,
            github,
            shutdown: CancellationToken::new(),
            daemon: None,
            clock: TickingClock::new(),
        }
    }

    fn setup(&self) -> DaemonSetup {
        DaemonSetup {
            home: self.home.clone(),
            prompts_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            clock: SharedClock::new(self.clock.clone()),
            local_offset: time::UtcOffset::UTC,
            timings: Some(Timings {
                tick: Duration::from_millis(25),
                run: RunTimings {
                    poll: Duration::from_millis(20),
                    hang: Duration::from_secs(20),
                    stop_grace: Duration::from_secs(5),
                },
            }),
        }
    }

    /// Starts the daemon and waits until its socket is there. Another test of this binary may be starting a process:
    /// a child holds the open file description of the lock of a daemon that has just stopped from its fork to its
    /// exec, so a daemon that finds the home locked is started again for a few milliseconds more.
    pub(super) async fn start(&mut self) {
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
    pub(super) async fn stop(&mut self) -> Result<(), DaemonError> {
        self.shutdown.cancel();
        let daemon = self.daemon.take().unwrap();
        tokio::time::timeout(PATIENCE, daemon).await.unwrap().unwrap()
    }

    /// Asks the daemon to run `agents` on `list` in the scratch repository with `merge`.
    pub(super) async fn run(
        &self,
        agents: &[&str],
        list: &str,
        merge: MergeMode,
        coordinator: Option<&str>,
    ) -> (TeamId, Vec<TaskId>) {
        let names: Vec<AgentName> = agents.iter().map(|name| name.parse().unwrap()).collect();
        let request = RunRequest {
            repo: "example".parse().unwrap(),
            checkout: abs(&self.repo),
            agents: AgentList::try_from(names).unwrap(),
            task_list: TaskListText::new(list.to_owned()).unwrap(),
            coordinator: coordinator.map(|name| name.parse().unwrap()),
            merge,
        };
        let reply = Connection::connect(&self.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, Command::Run(request))
            .await
            .unwrap();
        let Reply::Started { team, tasks } = reply else {
            panic!("a team was expected to start: {reply:?}")
        };
        (team, tasks)
    }

    /// The state as saved.
    pub(super) fn state(&self) -> State {
        match std::fs::read_to_string(self.home.state_file()) {
            Ok(text) => State::from_json(&text).unwrap(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => State::new(),
            Err(error) => panic!("{error}"),
        }
    }

    /// Waits until the saved state satisfies `ready`.
    pub(super) async fn state_where(&self, what: &str, ready: impl Fn(&State) -> bool) -> State {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Ok(text) = std::fs::read_to_string(self.home.state_file())
                && let Ok(state) = State::from_json(&text)
                && ready(&state)
            {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}\nstate: {}\nrequests: {:#?}",
                std::fs::read_to_string(self.home.state_file()).unwrap_or_default(),
                self.github.requests()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until `ready` holds.
    pub(super) async fn wait_for(&self, what: &str, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}\nstate: {}\nnotifications: {:?}",
                std::fs::read_to_string(self.home.state_file()).unwrap_or_default(),
                self.notifications()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until the item `id` of the only team is in the phase `phase` says.
    pub(super) async fn item_where(&self, id: &str, what: &str, ready: impl Fn(&ItemPhase) -> bool) -> State {
        let item = id.parse().unwrap();
        self.state_where(&format!("item {id} to be {what}"), |state| {
            state
                .teams()
                .first()
                .and_then(|team| team.item(item))
                .is_some_and(|progress| ready(progress.phase()))
        })
        .await
    }

    /// Waits until the task has the status.
    pub(super) async fn status_is(&self, task: &TaskId, status: TaskStatus) -> State {
        self.state_where(&format!("{task} to be {status}"), |state| {
            state
                .find_task(task)
                .is_some_and(|found| found.member.status() == status)
        })
        .await
    }

    /// The worktree of a task.
    pub(super) fn worktree(&self, task: &TaskId) -> PathBuf {
        self.home.worktree_dir(&"example".parse().unwrap(), task)
    }

    /// How many runs the task has had.
    pub(super) fn runs(&self, task: &TaskId) -> u32 {
        std::fs::read_to_string(self.worktree(task).join(".fake-runs")).map_or(0, |text| text.trim().parse().unwrap())
    }

    /// The prompt the task's `n`-th run was given, once it has been.
    pub(super) async fn prompt(&self, task: &TaskId, n: u32) -> String {
        let log = self.worktree(task).join(".fake-log");
        let wanted = usize::try_from(n).unwrap();
        self.wait_for(&format!("run {n} of {task} to be logged"), || {
            std::fs::read_to_string(&log).is_ok_and(|text| text.lines().count() >= wanted)
        })
        .await;
        std::fs::read_to_string(self.worktree(task).join(format!(".fake-prompt-{n}"))).unwrap()
    }

    /// The notifications written so far.
    pub(super) fn notifications(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.home.notifications_dir()) else {
            return Vec::new();
        };
        let mut texts: Vec<String> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_none_or(|extension| extension != "tmp"))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect();
        texts.sort();
        texts
    }

    /// Waits for a notification that contains `needle` and returns it.
    pub(super) async fn notification_with(&self, needle: &str) -> String {
        self.wait_for(&format!("a notification with {needle:?}"), || {
            self.notifications().iter().any(|text| text.contains(needle))
        })
        .await;
        self.notifications()
            .into_iter()
            .find(|text| text.contains(needle))
            .unwrap()
    }

    /// The tip of `branch` in the remote.
    pub(super) fn remote_tip(&self, branch: &str) -> String {
        git(&self.origin, &["rev-parse", &format!("refs/heads/{branch}")])
    }
}

/// The configuration of a home with these agents, the repository `example` and the stand-in CLI for every agent.
fn config(agents: &[Agent], github: &str, fix_passes: u32, home: &Home, bus: bool) -> String {
    let mut text = String::new();
    let mut normals = Vec::new();
    for agent in agents {
        let table = if agent.investigator { "investigators" } else { "agents" };
        let mut args = format!(
            "\"--scenario\", \"{}\", \"--as\", \"{}\", \"--home\", \"{}\"",
            // A backslash of a scenario is the script's (`\n` in the text of an action), not an escape of the TOML file.
            agent.scenario.replace('\\', "\\\\"),
            agent.name,
            home.root().display()
        );
        if bus {
            write!(
                args,
                ", \"--bus-bin\", \"{}\", \"--bus-dir\", \"{}/bus\"",
                super::super::fixture::agentbus_binary().display(),
                home.root().display()
            )
            .unwrap();
        }
        write!(text, "[{table}.{}]\ncli = \"claude\"\nargs = [{args}]\n", agent.name).unwrap();
        if !agent.investigator {
            normals.push(format!("\"{}\"", agent.name));
        }
    }
    write!(
        text,
        "[tiers]\nnormal = [{}]\n[binaries]\nclaude = \"{}\"\nagentbus = \"{}\"\n[watchdog]\nhang_minutes = 1\n\
         hang_strikes = 3\n[review]\nfix_passes = {fix_passes}\n[github]\napi_base = \"{github}\"\n\
         trusted_accounts = [\"{ACCOUNT}\"]\n[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
        normals.join(", "),
        fixture_script("team-cli.sh").display(),
        super::super::fixture::agentbus_binary().display(),
    )
    .unwrap();
    text
}
