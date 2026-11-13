//! A daemon in a temporary home with a stand-in for the `claude` CLI, for the tests of what the human does to it:
//! answering questions and the controls.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use agent_core::config::AbsolutePath;
use agent_core::protocol::{
    AnswerInput, AnswerList, AnswerRequest, Client, Command, Event, Frame, Reply, Request, RequestId, SubscribeRequest,
};
use agent_core::questions::Verdict;
use agent_core::state::{Crew, Member, MergeMode, State, TaskListText, Team, TeamMembers};
use agent_core::status::TaskStatus;
use agent_runtime::clock::SharedClock;
use agent_runtime::run::RunTimings;
use agent_runtime::store::Home;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::client::{ClientError, Connection};
use crate::daemon::{DaemonError, DaemonSetup, Timings, run};
use crate::fixtures::{at, member, normal, repo, task, team_id};
use crate::testing::ManualClock;

/// The longest any wait of these tests lasts.
pub(super) const PATIENCE: Duration = Duration::from_secs(30);

/// The fake CLI, the one of the tests of the run engine.
fn fake_cli() -> String {
    format!(
        "{}/../agent-runtime/tests/fixtures/run/fake-claude.sh",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The `agentbus` built from `agentbus/`, once for all the tests that need a bus.
pub(super) fn agentbus_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus");
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        std::fs::create_dir_all(&target).unwrap();
        let binary = target.join("agentbus-under-test-of-steering");
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

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

/// A question number from a literal.
pub(super) fn n(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

/// The answers of a request: a verdict for each question given.
pub(super) fn answers(given: &[(u32, &str)]) -> AnswerList {
    AnswerList::try_from(
        given
            .iter()
            .map(|(question, text)| AnswerInput {
                question: n(*question),
                verdict: Verdict::from_text(text).unwrap().unwrap(),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

/// The request that answers round `round` of task `task_number`.
pub(super) fn answer(task_number: u32, round: u32, given: &[(u32, &str)]) -> Command {
    Command::Answer(AnswerRequest {
        task: task(task_number),
        round: n(round),
        answers: answers(given),
    })
}

/// A state with one team of a coordinator (task 1, `claude-1`) and a member (task 2, `claude-2`), on a task list
/// without items.
pub(super) fn pair_state() -> State {
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, normal("claude-1")),
        members: vec![Member::new(task(2), normal("claude-2"))],
        workers: vec![],
    })
    .unwrap();
    let team = Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Fix the typo in the README.\n".to_owned()).unwrap(),
        crew,
    );
    let mut state = State::new();
    state.insert_team(team).unwrap();
    state
}

/// A daemon in a temporary home.
pub(super) struct Daemon {
    _dir: tempfile::TempDir,
    pub(super) home: Home,
    pub(super) clock: ManualClock,
    shutdown: CancellationToken,
    daemon: Option<JoinHandle<Result<(), DaemonError>>>,
}

impl Daemon {
    /// A home with `state` and the configuration of two agents that run the fake CLI; `bus` is the `agentbus` to use.
    pub(super) fn new(state: &State, bus: Option<&Path>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(dir.path()));
        let bus_line = bus.map_or_else(String::new, |bus| format!("agentbus = \"{}\"\n", bus.display()));
        let config = format!(
            "[agents.claude-1]\ncli = \"claude\"\n[agents.claude-2]\ncli = \"claude\"\n\
             [tiers]\nnormal = [\"claude-1\", \"claude-2\"]\n[binaries]\nclaude = \"{}\"\n{bus_line}\
             [watchdog]\nhang_minutes = 1\nhang_strikes = 3\n\
             [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
            fake_cli()
        );
        std::fs::write(home.config_file(), config).unwrap();
        // Written directly: opening the store would hold its lock.
        std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
        Self {
            _dir: dir,
            home,
            clock: ManualClock::new(at(0)),
            shutdown: CancellationToken::new(),
            daemon: None,
        }
    }

    /// The worktree of task `n`, which makes the fake CLI do `scenario`.
    pub(super) fn scenario(&self, n: u32, scenario: &str) -> PathBuf {
        let dir = self.home.worktree_dir(&repo(), &task(n));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".fake-scenario"), scenario).unwrap();
        dir
    }

    /// The text of a file the fake CLI wrote in the worktree of task `n`.
    pub(super) fn recorded(&self, n: u32, name: &str) -> String {
        std::fs::read_to_string(self.home.worktree_dir(&repo(), &task(n)).join(name)).unwrap()
    }

    /// Starts the daemon and waits until its socket is there.
    ///
    /// A daemon that finds the home locked is started again after a moment: a process that another test forks at the very
    /// time the previous daemon ended keeps the lock a little longer than that daemon.
    pub(super) async fn start(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.try_start(deadline).await {
                Ok(daemon) => {
                    self.daemon = Some(daemon);
                    return;
                }
                Err(DaemonError::AlreadyRunning { .. }) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("the daemon ended at its start: {error:?}"),
            }
        }
    }

    /// Starts the daemon once; it is returned when its socket is there, and its error when it ends before.
    async fn try_start(&mut self, deadline: Instant) -> Result<JoinHandle<Result<(), DaemonError>>, DaemonError> {
        self.shutdown = CancellationToken::new();
        let setup = DaemonSetup {
            home: self.home.clone(),
            prompts_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            clock: SharedClock::new(self.clock.clone()),
            local_offset: time::UtcOffset::UTC,
            timings: Some(Timings {
                tick: Duration::from_millis(20),
                run: RunTimings {
                    poll: Duration::from_millis(20),
                    hang: Duration::from_secs(1),
                    stop_grace: Duration::from_secs(5),
                },
            }),
        };
        let daemon = tokio::spawn(run(setup, self.shutdown.clone()));
        let socket = self.home.socket_file();
        while !socket.exists() {
            if daemon.is_finished() {
                return Err(daemon
                    .await
                    .unwrap()
                    .expect_err("a daemon that is not asked to stop does not end well"));
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the socket of the daemon"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(daemon)
    }

    /// Stops the daemon and returns how it ended.
    pub(super) async fn stop(&mut self) -> Result<(), DaemonError> {
        self.shutdown.cancel();
        let daemon = self.daemon.take().unwrap();
        tokio::time::timeout(PATIENCE, daemon).await.unwrap().unwrap()
    }

    /// The state as saved.
    pub(super) fn state(&self) -> State {
        State::from_json(&std::fs::read_to_string(self.home.state_file()).unwrap()).unwrap()
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
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until task `n` has `status`.
    pub(super) async fn status_is(&self, n: u32, status: TaskStatus) -> State {
        self.state_where(&format!("task {n} to be {status}"), |state| {
            state
                .find_task(&task(n))
                .is_some_and(|found| found.member.status() == status)
        })
        .await
    }

    /// A connection to the daemon.
    pub(super) async fn connect(&self) -> Connection {
        Connection::connect(&self.home.socket_file()).await.unwrap()
    }

    /// Sends `command` as the command line and returns the reply.
    pub(super) async fn ask(&self, command: Command) -> Result<Reply, ClientError> {
        self.connect().await.request(Client::Cli, command).await
    }

    /// The text of the notifications written so far.
    pub(super) fn notifications(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.home.notifications_dir()) else {
            return Vec::new();
        };
        let mut texts: Vec<String> = entries
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        texts.sort();
        texts
    }

    /// Waits until a notification that contains `needle` is written, and returns it.
    pub(super) async fn notification_with(&self, needle: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = self.notifications().into_iter().find(|text| text.contains(needle)) {
                return found;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a notification with {needle:?}: {:?}",
                self.notifications()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The lines of the activity log of task `n`.
    pub(super) fn activity(&self, n: u32) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.home.activity_log(&task(n)))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// The questions file of round `round` of task `n`.
    pub(super) fn questions_file(&self, n: u32, round: u32) -> PathBuf {
        self.home.questions_file(&task(n), self::n(round))
    }

    /// Subscribes to the events of every team.
    pub(super) async fn subscribe(&self) -> Events {
        let (read, mut writer) = tokio::net::UnixStream::connect(self.home.socket_file())
            .await
            .unwrap()
            .into_split();
        let mut reader = tokio::io::BufReader::new(read);
        let line = Request::new(
            RequestId(1),
            Client::Cli,
            Command::Subscribe(SubscribeRequest { team: None }),
        )
        .to_line()
        .unwrap();
        crate::lines::write_line(&mut writer, &line).await.unwrap();
        // The answer to the subscription comes first.
        let first = crate::lines::read_line(&mut reader, 1 << 24).await.unwrap().unwrap();
        assert!(
            matches!(Frame::from_line(&first).unwrap(), Frame::Response(_)),
            "{first}"
        );
        Events {
            reader,
            _writer: writer,
        }
    }
}

/// The events of a subscription.
pub(super) struct Events {
    reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    _writer: tokio::net::unix::OwnedWriteHalf,
}

impl Events {
    /// Reads events until `found` picks one out of them.
    pub(super) async fn until<T>(&mut self, what: &str, found: impl Fn(&Event) -> Option<T>) -> T {
        loop {
            let line = tokio::time::timeout(PATIENCE, crate::lines::read_line(&mut self.reader, 1 << 24))
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .unwrap()
                .unwrap_or_else(|| panic!("the connection ended while waiting for {what}"));
            if let Frame::Event(event) = Frame::from_line(&line).unwrap()
                && let Some(done) = found(&event)
            {
                return done;
            }
        }
    }
}
