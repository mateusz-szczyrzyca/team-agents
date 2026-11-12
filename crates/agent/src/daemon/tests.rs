//! The daemon in a temporary home, with a stand-in for the `claude` CLI: how tasks run to their end, what is recorded
//! and told, the watchdog, the pauses of limits, recovery after a restart, the shutdown and the socket.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use agent_core::config::AbsolutePath;
use agent_core::protocol::{Client, Command, FailureKind, Reply, ShowRequest, StatusRequest, SubscribeRequest, Target};
use agent_core::state::{RunEnd, RunState, State, Timestamp};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_runtime::clock::SharedClock;
use agent_runtime::run::RunTimings;
use agent_runtime::store::{Home, StateStore, StoreError};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::client::{ClientError, Connection};
use crate::fixtures::{at, member, normal, repo, run as run_id, solo_state, task};
use crate::testing::ManualClock;

/// The longest any wait of these tests lasts.
const PATIENCE: Duration = Duration::from_secs(30);

/// The fake CLI, the one of the tests of the run engine.
fn fake_cli() -> String {
    format!(
        "{}/../agent-runtime/tests/fixtures/run/fake-claude.sh",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn config(hang_strikes: u32) -> String {
    format!(
        "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{}\"\n\
         [watchdog]\nhang_minutes = 1\nhang_strikes = {hang_strikes}\n\
         [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
        fake_cli()
    )
}

fn timings() -> Timings {
    Timings {
        tick: Duration::from_millis(20),
        run: RunTimings {
            poll: Duration::from_millis(20),
            hang: Duration::from_secs(1),
            stop_grace: Duration::from_secs(5),
        },
    }
}

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

/// A daemon in a temporary home.
struct Fixture {
    _dir: tempfile::TempDir,
    home: Home,
    clock: ManualClock,
    shutdown: CancellationToken,
    daemon: Option<JoinHandle<Result<(), DaemonError>>>,
}

impl Fixture {
    /// A home with `state`, the configuration of [`config`] and a worktree for task 1 that runs `scenario`.
    fn new(state: &State, hang_strikes: u32, scenario: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(dir.path()));
        std::fs::write(home.config_file(), config(hang_strikes)).unwrap();
        // Written directly: opening the store would hold its lock, which a process that is forked at that moment
        // keeps a little longer than the test.
        std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
        let fixture = Self {
            _dir: dir,
            home,
            clock: ManualClock::new(at(0)),
            shutdown: CancellationToken::new(),
            daemon: None,
        };
        fixture.worktree(1, scenario);
        fixture
    }

    /// The worktree of task `n`, which makes the fake CLI do `scenario`.
    fn worktree(&self, n: u32, scenario: &str) -> PathBuf {
        let dir = self.home.worktree_dir(&repo(), &task(n));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".fake-scenario"), scenario).unwrap();
        dir
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

    /// Starts the daemon and waits until its socket is there.
    async fn start(&mut self) {
        let daemon = tokio::spawn(run(self.setup(), self.shutdown.clone()));
        let socket = self.home.socket_file();
        let deadline = Instant::now() + PATIENCE;
        while !socket.exists() {
            assert!(
                !daemon.is_finished(),
                "the daemon ended at its start: {:?}",
                daemon.await
            );
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the socket of the daemon"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.daemon = Some(daemon);
    }

    /// Stops the daemon and returns how it ended.
    async fn stop(&mut self) -> Result<(), DaemonError> {
        self.shutdown.cancel();
        let daemon = self.daemon.take().unwrap();
        tokio::time::timeout(PATIENCE, daemon).await.unwrap().unwrap()
    }

    /// The state as saved.
    fn state(&self) -> State {
        State::from_json(&std::fs::read_to_string(self.home.state_file()).unwrap()).unwrap()
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

    /// Waits until task 1 has this status.
    async fn status_is(&self, status: TaskStatus) -> State {
        self.state_where(&format!("the status {status}"), |state| {
            state
                .find_task(&task(1))
                .is_some_and(|found| found.member.status() == status)
        })
        .await
    }

    /// The text of the notifications written so far.
    fn notifications(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.home.notifications_dir()) else {
            return Vec::new();
        };
        let mut texts: Vec<String> = entries
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        texts.sort();
        texts
    }

    fn recorded(&self, name: &str) -> String {
        std::fs::read_to_string(self.home.worktree_dir(&repo(), &task(1)).join(name)).unwrap()
    }

    async fn connect(&self) -> Connection {
        Connection::connect(&self.home.socket_file()).await.unwrap()
    }
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn member_one(state: &State) -> &agent_core::state::Member {
    state.find_task(&task(1)).unwrap().member
}

fn end_of(run: &agent_core::state::Run) -> &RunEnd {
    match run.state() {
        RunState::Ended { end, .. } => end,
        RunState::Running => panic!("the run is still going"),
    }
}

fn is_gone(pid: i32) -> bool {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    text.rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        .is_none_or(|state| matches!(state, 'Z' | 'X'))
}

// ---- the life of a task ------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_queued_solo_task_runs_to_done() {
    let mut f = Fixture::new(&solo_state(), 3, "done");

    f.start().await;
    let state = f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();

    let member = member_one(&state);
    assert_eq!(member.runs().len(), 1);
    let run = &member.runs()[0];
    assert_eq!(
        end_of(run),
        &RunEnd::Reported {
            outcome: RunOutcome::Done
        }
    );
    assert!(run.session().is_some(), "the session of the run is kept");
    assert!(f.recorded(".fake-args").contains("--session-id"));
}

#[tokio::test]
async fn the_first_prompt_has_the_context_the_role_and_the_rules() {
    let mut f = Fixture::new(&solo_state(), 3, "done");

    f.start().await;
    f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();

    let args = f.recorded(".fake-args");
    assert!(args.contains("You are agent **claude-1**"), "{args}");
    assert!(args.contains(&format!("Task: `{}`", task(1))), "{args}");
    assert!(args.contains("Your role: solo"), "{args}");
    assert!(args.contains("Rust developer rules"), "{args}");
    assert!(args.contains("Build the thing."), "the brief of the task list: {args}");
    assert!(
        args.contains(
            f.home
                .questions_file(&task(1), std::num::NonZeroU32::MIN)
                .to_str()
                .unwrap()
        ),
        "the questions file of the first round: {args}"
    );
}

#[tokio::test]
async fn the_activity_of_the_run_is_in_the_task_directory() {
    let mut f = Fixture::new(&solo_state(), 3, "done");

    f.start().await;
    f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();

    let log = std::fs::read_to_string(f.home.activity_log(&task(1))).unwrap();
    assert!(log.contains("\"working\""), "{log}");
}

#[tokio::test]
async fn a_run_that_asks_questions_records_the_round_and_tells_the_human() {
    let mut f = Fixture::new(&solo_state(), 3, "done");
    let file = f.home.questions_file(&task(1), std::num::NonZeroU32::MIN);
    f.worktree(1, &format!("questions {}", file.display()));

    f.start().await;
    let state = f.status_is(TaskStatus::Questions).await;
    f.stop().await.unwrap();

    let question = member_one(&state).open_question().unwrap();
    assert_eq!(question.round().get(), 1);
    assert_eq!(question.file().as_path(), file);
    assert!(file.is_file());
    let notes = f.notifications();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("claude-1 has questions"), "{}", notes[0]);
    assert!(notes[0].contains(file.to_str().unwrap()), "{}", notes[0]);
}

#[tokio::test]
async fn questions_without_a_file_fail_the_task_and_say_why() {
    let mut f = Fixture::new(&solo_state(), 3, "questions-nofile");

    f.start().await;
    let state = f.status_is(TaskStatus::Failed).await;
    f.stop().await.unwrap();

    assert_eq!(member_one(&state).questions().len(), 0);
    let notes = f.notifications();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("questions that cannot be read"), "{}", notes[0]);
}

#[tokio::test]
async fn a_reported_failure_and_a_missing_status_fail_the_task_and_tell_the_human() {
    for (scenario, says) in [("fail", "I could not do it."), ("nostatus", "without a status")] {
        let mut f = Fixture::new(&solo_state(), 3, scenario);

        f.start().await;
        f.status_is(TaskStatus::Failed).await;
        f.stop().await.unwrap();

        let notes = f.notifications();
        assert_eq!(notes.len(), 1, "{scenario}: {notes:?}");
        assert!(notes[0].contains(says), "{scenario}: {}", notes[0]);
    }
}

// ---- the watchdog ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_hung_run_is_resumed_in_its_session() {
    let mut f = Fixture::new(&solo_state(), 3, "hang-first");

    f.start().await;
    let state = f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();

    let runs = member_one(&state).runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(end_of(&runs[0]), &RunEnd::Hung);
    assert_eq!(
        end_of(&runs[1]),
        &RunEnd::Reported {
            outcome: RunOutcome::Done
        }
    );
    let session = runs[0].session().unwrap();
    let args = f.recorded(".fake-args");
    assert!(args.contains(&format!("--resume\n{session}")), "{args}");
    assert!(args.contains("A HUNG OPERATION WAS KILLED"), "{args}");
    assert!(args.contains("1 minutes"), "the limit of the configuration: {args}");
}

#[tokio::test]
async fn after_the_configured_strikes_the_task_is_hung_and_the_human_is_told() {
    let mut f = Fixture::new(&solo_state(), 2, "hang");

    f.start().await;
    let state = f.status_is(TaskStatus::Failed).await;
    f.stop().await.unwrap();

    let runs = member_one(&state).runs();
    assert_eq!(runs.len(), 2, "one run, one resume, then the task is hung");
    assert!(runs.iter().all(|run| end_of(run) == &RunEnd::Hung));
    let notes = f.notifications();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("claude-1 is hung"), "{}", notes[0]);
    assert!(notes[0].contains("2 times"), "{}", notes[0]);
}

#[tokio::test]
async fn a_clock_set_back_during_a_run_does_not_end_the_daemon() {
    let mut f = Fixture::new(&solo_state(), 1, "hang");
    f.start().await;
    let state = f.status_is(TaskStatus::Running).await;
    let started_at = member_one(&state).runs()[0].started_at();

    f.clock.advance(time::Duration::hours(-1));
    let state = f.status_is(TaskStatus::Failed).await;
    f.stop().await.unwrap();

    let run = &member_one(&state).runs()[0];
    assert_eq!(
        run.state(),
        &RunState::Ended {
            at: started_at,
            end: RunEnd::Hung
        }
    );
}

// ---- limits ------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_exhausted_quota_pauses_the_task_until_the_reset_and_it_goes_on_by_itself() {
    let mut f = Fixture::new(&solo_state(), 3, "quota");

    f.start().await;
    let state = f.status_is(TaskStatus::Paused).await;

    let run = &member_one(&state).runs()[0];
    let reset = Timestamp::from_utc(at(0).as_utc() + time::Duration::hours(3)).unwrap();
    assert_eq!(end_of(run), &RunEnd::QuotaExhausted { resets_at: Some(reset) });
    let notes = f.notifications();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("is paused until 2026-11-11T22:16:00Z"),
        "{}",
        notes[0]
    );
    // Time passes, and the provider lets the agent go on.
    f.worktree(1, "done");
    f.clock.advance(time::Duration::hours(4));
    let state = f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();
    let runs = member_one(&state).runs();
    assert_eq!(runs.len(), 2);
    let args = f.recorded(".fake-args");
    assert!(args.contains("--resume"), "{args}");
    assert!(args.contains("THE LIMIT OF YOUR MODEL'S PROVIDER IS OVER"), "{args}");
}

#[tokio::test]
async fn a_task_that_is_paused_is_not_started_before_its_time() {
    let mut f = Fixture::new(&solo_state(), 3, "ratelimit");

    f.start().await;
    f.status_is(TaskStatus::Paused).await;
    f.worktree(1, "done");
    f.clock.advance(time::Duration::minutes(5));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let state = f.state();
    assert_eq!(member_one(&state).status(), TaskStatus::Paused);
    assert_eq!(member_one(&state).runs().len(), 1);
    f.stop().await.unwrap();
}

// ---- restarts ----------------------------------------------------------------------------------------------------

/// A state in which task 1 was running (run 1, session `ses-1`) when the daemon died.
fn crashed_state() -> State {
    let mut state = solo_state();
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(run_id(1), at(5), Some("ses-1".parse().unwrap()))
        .unwrap();
    state
}

#[tokio::test]
async fn a_run_that_was_going_when_the_daemon_died_is_marked_and_resumed_from_its_session() {
    let mut f = Fixture::new(&crashed_state(), 3, "done");

    f.start().await;
    let state = f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();

    let runs = member_one(&state).runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(end_of(&runs[0]), &RunEnd::NoStatus { exit_code: None });
    assert_eq!(
        end_of(&runs[1]),
        &RunEnd::Reported {
            outcome: RunOutcome::Done
        }
    );
    let args = f.recorded(".fake-args");
    assert!(args.contains("--resume\nses-1"), "{args}");
    assert!(args.contains("THE HARNESS RESTARTED"), "{args}");
}

#[tokio::test]
async fn a_pause_survives_a_restart_and_ends_on_time() {
    let mut state = solo_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run(run_id(1), at(5), Some("ses-1".parse().unwrap()))
            .unwrap();
        member.end_run(at(10), RunEnd::RateLimited).unwrap();
        member.transition(TaskStatus::Paused).unwrap();
    }
    let mut f = Fixture::new(&state, 3, "done");
    f.start().await;
    f.status_is(TaskStatus::Paused).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        member_one(&f.state()).status(),
        TaskStatus::Paused,
        "the pause is not over yet"
    );

    f.clock.advance(time::Duration::minutes(10));
    let state = f.status_is(TaskStatus::Done).await;
    f.stop().await.unwrap();

    assert_eq!(member_one(&state).runs().len(), 2);
    assert!(f.recorded(".fake-args").contains("--resume\nses-1"));
}

// ---- shutdown ----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_shutdown_stops_the_runs_whole_and_records_them_as_interrupted() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    let marker = f.home.worktree_dir(&repo(), &task(1)).join(".fake-child");
    eventually("the child of the run", || marker.is_file()).await;
    let child: i32 = f.recorded(".fake-child").trim().parse().unwrap();

    f.stop().await.unwrap();

    eventually("the child of the run to be gone", || is_gone(child)).await;
    let state = f.state();
    let member = member_one(&state);
    assert_eq!(member.status(), TaskStatus::Queued);
    assert_eq!(end_of(&member.runs()[0]), &RunEnd::NoStatus { exit_code: None });
    assert!(!f.home.socket_file().exists(), "the socket is removed");
}

#[tokio::test]
async fn a_second_daemon_on_the_same_home_is_refused() {
    let mut f = Fixture::new(&solo_state(), 3, "done");
    f.start().await;

    let error = run(f.setup(), CancellationToken::new()).await.unwrap_err();

    assert!(
        matches!(&error, DaemonError::AlreadyRunning { path } if *path == f.home.lock_file()),
        "{error:?}"
    );
    assert!(error.to_string().contains("already running"), "{error}");
    f.stop().await.unwrap();
    // The lock is released: a new store opens (a process forked meanwhile may keep the lock for a moment).
    let deadline = Instant::now() + PATIENCE;
    while matches!(
        StateStore::open(f.home.clone()).await,
        Err(StoreError::HomeLocked { .. })
    ) {
        assert!(Instant::now() < deadline, "the lock of the daemon is still held");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_home_without_a_configuration_is_refused_with_the_file_named() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&abs(dir.path()));
    let setup = DaemonSetup {
        home: home.clone(),
        prompts_dir: home.prompts_dir(),
        clock: SharedClock::new(ManualClock::new(at(0))),
        local_offset: time::UtcOffset::UTC,
        timings: None,
    };

    let error = run(setup, CancellationToken::new()).await.unwrap_err();

    assert!(matches!(error, DaemonError::Settings(_)), "{error:?}");
}

#[tokio::test]
async fn a_prompts_directory_that_lacks_files_is_refused_with_every_problem() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&abs(dir.path()));
    std::fs::write(home.config_file(), config(3)).unwrap();
    let setup = DaemonSetup {
        home: home.clone(),
        prompts_dir: home.prompts_dir(),
        clock: SharedClock::new(ManualClock::new(at(0))),
        local_offset: time::UtcOffset::UTC,
        timings: None,
    };

    let error = run(setup, CancellationToken::new()).await.unwrap_err();

    assert!(
        matches!(error, DaemonError::HarnessPrompts(_) | DaemonError::Prompts(_)),
        "{error:?}"
    );
}

// ---- tasks that cannot be started ---------------------------------------------------------------------------------

#[tokio::test]
async fn a_task_without_a_worktree_fails_and_the_human_is_told() {
    let mut f = Fixture::new(&solo_state(), 3, "done");
    std::fs::remove_dir_all(f.home.worktree_dir(&repo(), &task(1))).unwrap();

    f.start().await;
    let state = f.status_is(TaskStatus::Failed).await;
    f.stop().await.unwrap();

    assert_eq!(member_one(&state).runs().len(), 0);
    let notes = f.notifications();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("could not be started"), "{}", notes[0]);
    assert!(notes[0].contains("worktree"), "{}", notes[0]);
}

#[tokio::test]
async fn an_agent_the_configuration_no_longer_has_fails_its_task() {
    let mut state = solo_state();
    let team = state.teams()[0].id().clone();
    let crew = agent_core::state::Crew::solo(member(1, normal("gone-1"))).unwrap();
    let replaced = agent_core::state::Team::new(
        team,
        repo(),
        at(0),
        agent_core::state::MergeMode::Autopilot,
        agent_core::state::TaskListText::new(crate::fixtures::LIST.to_owned()).unwrap(),
        crew,
    );
    state = State::new();
    state.insert_team(replaced).unwrap();
    let mut f = Fixture::new(&state, 3, "done");

    f.start().await;
    f.status_is(TaskStatus::Failed).await;
    f.stop().await.unwrap();

    let notes = f.notifications();
    assert!(notes[0].contains("gone-1"), "{}", notes[0]);
}

// ---- the socket --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_socket_is_for_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new(&solo_state(), 3, "hang");

    f.start().await;

    let mode = std::fs::metadata(f.home.socket_file()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    f.stop().await.unwrap();
}

#[tokio::test]
async fn a_socket_file_left_by_a_dead_daemon_is_replaced() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    drop(tokio::net::UnixListener::bind(f.home.socket_file()).unwrap());
    assert!(f.home.socket_file().exists());

    f.daemon = Some(tokio::spawn(run(f.setup(), f.shutdown.clone())));
    let deadline = Instant::now() + PATIENCE;
    let mut connection = loop {
        if let Ok(connection) = Connection::connect(&f.home.socket_file()).await {
            break connection;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the new socket");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();

    assert!(matches!(reply, Reply::Teams { .. }));
    f.stop().await.unwrap();
}

#[tokio::test]
async fn status_over_the_socket_lists_the_teams_with_their_tasks() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    f.status_is(TaskStatus::Running).await;
    let mut connection = f.connect().await;

    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();

    let Reply::Teams { teams } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].members.len(), 1);
    assert_eq!(teams[0].members[0].task, task(1));
    assert_eq!(teams[0].members[0].status, TaskStatus::Running);
    f.stop().await.unwrap();
}

#[tokio::test]
async fn status_of_a_team_or_a_task_names_only_its_team_and_an_unknown_one_is_not_found() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    let mut connection = f.connect().await;
    let team = f.state().teams()[0].id().clone();

    let by_team = connection
        .request(
            Client::Cli,
            Command::Status(StatusRequest {
                target: Some(Target::Team(team.clone())),
            }),
        )
        .await
        .unwrap();
    let by_task = connection
        .request(
            Client::Cli,
            Command::Status(StatusRequest {
                target: Some(Target::Task(task(1))),
            }),
        )
        .await
        .unwrap();
    let unknown = connection
        .request(
            Client::Cli,
            Command::Status(StatusRequest {
                target: Some(Target::Task(task(9))),
            }),
        )
        .await
        .unwrap_err();

    assert!(matches!(&by_team, Reply::Teams { teams } if teams.len() == 1 && teams[0].id == team));
    assert!(matches!(&by_task, Reply::Teams { teams } if teams.len() == 1 && teams[0].id == team));
    assert!(
        matches!(&unknown, ClientError::Refused(failure) if failure.kind == FailureKind::NotFound),
        "{unknown:?}"
    );
    f.stop().await.unwrap();
}

#[tokio::test]
async fn show_gives_the_whole_team_or_the_task_with_its_role() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    let mut connection = f.connect().await;
    let team = f.state().teams()[0].id().clone();

    let whole = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Team(team.clone()),
            }),
        )
        .await
        .unwrap();
    let one = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Task(task(1)),
            }),
        )
        .await
        .unwrap();

    assert!(matches!(&whole, Reply::Team { team: shown } if *shown.id() == team));
    assert!(
        matches!(&one, Reply::Task { team: id, role, member } if *id == team && *role == agent_core::kinds::Role::Solo && *member.task() == task(1)),
        "{one:?}"
    );
    f.stop().await.unwrap();
}

#[tokio::test]
async fn commands_the_daemon_does_not_serve_yet_are_refused_with_their_name() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    let mut connection = f.connect().await;

    let error = connection
        .request(
            Client::Cli,
            Command::Stop(agent_core::protocol::ControlRequest {
                target: Target::Task(task(1)),
            }),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(&error, ClientError::Refused(failure) if failure.kind == FailureKind::Refused && failure.message.contains("stop")),
        "{error:?}"
    );
    f.stop().await.unwrap();
}

/// A connection on which lines are written and read as they are, for what the client does not do.
struct RawConnection {
    reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl RawConnection {
    async fn open(home: &Home) -> Self {
        let (read, writer) = tokio::net::UnixStream::connect(home.socket_file())
            .await
            .unwrap()
            .into_split();
        Self {
            reader: tokio::io::BufReader::new(read),
            writer,
        }
    }

    async fn send(&mut self, line: &str) {
        crate::lines::write_line(&mut self.writer, line).await.unwrap();
    }

    async fn frame(&mut self) -> Option<agent_core::protocol::Frame> {
        let line = tokio::time::timeout(PATIENCE, crate::lines::read_line(&mut self.reader, 1 << 24))
            .await
            .unwrap()
            .unwrap()?;
        Some(agent_core::protocol::Frame::from_line(&line).unwrap())
    }
}

fn request_line(id: u64, command: Command) -> String {
    agent_core::protocol::Request::new(agent_core::protocol::RequestId(id), Client::Cli, command)
        .to_line()
        .unwrap()
}

/// A state in which task 1 is paused by a rate limit that ended its run 5 seconds after 19:15:00.
fn rate_limited_state() -> State {
    let mut state = solo_state();
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(run_id(1), at(5), Some("ses-1".parse().unwrap()))
        .unwrap();
    member.end_run(at(10), RunEnd::RateLimited).unwrap();
    member.transition(TaskStatus::Paused).unwrap();
    state
}

#[tokio::test]
async fn a_subscriber_gets_the_teams_and_then_the_events_of_what_happens() {
    use agent_core::protocol::{EventKind, Frame};
    let mut f = Fixture::new(&rate_limited_state(), 3, "done");
    f.start().await;
    f.status_is(TaskStatus::Paused).await;
    let mut connection = RawConnection::open(&f.home).await;

    connection
        .send(&request_line(7, Command::Subscribe(SubscribeRequest { team: None })))
        .await;
    let Some(Frame::Response(response)) = connection.frame().await else {
        panic!("no response")
    };
    let agent_core::protocol::Outcome::Ok {
        reply: Reply::Subscribed { teams },
    } = response.result
    else {
        panic!("{response:?}")
    };
    assert_eq!(response.id.0, 7);
    assert_eq!(teams[0].members[0].status, TaskStatus::Paused);
    f.clock.advance(time::Duration::minutes(10));
    let mut seen = Vec::new();
    while let Some(Frame::Event(event)) = connection.frame().await {
        let finished = matches!(
            &event.event,
            EventKind::TaskStatus {
                status: TaskStatus::Done,
                ..
            }
        );
        seen.push(event.event);
        if finished {
            break;
        }
    }
    f.stop().await.unwrap();

    let kinds: Vec<&str> = seen
        .iter()
        .filter_map(|event| match event {
            EventKind::TaskStatus {
                status: TaskStatus::Queued,
                ..
            } => Some("queued"),
            EventKind::RunStarted { .. } => Some("run_started"),
            EventKind::TaskStatus {
                status: TaskStatus::Running,
                ..
            } => Some("running"),
            EventKind::RunEnded {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
                ..
            } => Some("run_ended"),
            EventKind::TaskStatus {
                status: TaskStatus::Done,
                ..
            } => Some("done"),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        ["queued", "run_started", "running", "run_ended", "done"],
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|event| matches!(event, EventKind::TeamUpdated { .. })),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_subscription_to_another_team_gets_no_events_of_this_one() {
    use agent_core::protocol::{EventKind, Frame};
    let mut f = Fixture::new(&rate_limited_state(), 3, "done");
    f.start().await;
    f.status_is(TaskStatus::Paused).await;
    let mut other = RawConnection::open(&f.home).await;
    let mut own = RawConnection::open(&f.home).await;
    let elsewhere = crate::fixtures::team_id(9);
    other
        .send(&request_line(
            1,
            Command::Subscribe(SubscribeRequest { team: Some(elsewhere) }),
        ))
        .await;
    own.send(&request_line(1, Command::Subscribe(SubscribeRequest { team: None })))
        .await;
    other.frame().await.unwrap();
    own.frame().await.unwrap();

    f.clock.advance(time::Duration::minutes(10));
    f.status_is(TaskStatus::Done).await;
    // Everything the run caused has reached the subscriber to this team; the other one has heard nothing.
    let mut got = false;
    while let Some(Frame::Event(event)) = own.frame().await {
        if matches!(
            event.event,
            EventKind::TaskStatus {
                status: TaskStatus::Done,
                ..
            }
        ) {
            got = true;
            break;
        }
    }
    assert!(got);
    f.stop().await.unwrap();
    assert!(
        other.frame().await.is_none(),
        "the connection ends with the daemon and carried no event"
    );
}

#[tokio::test]
async fn a_line_that_is_not_a_request_is_answered_with_an_error_and_the_connection_goes_on() {
    use agent_core::protocol::{Frame, Outcome};
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    let mut connection = RawConnection::open(&f.home).await;

    connection.send("this is not json\n").await;
    let Some(Frame::Response(refused)) = connection.frame().await else {
        panic!("no response")
    };
    connection.send("{\"protocol\":9,\"id\":3}\n").await;
    let Some(Frame::Response(foreign)) = connection.frame().await else {
        panic!("no response")
    };
    connection
        .send(&request_line(4, Command::Status(StatusRequest { target: None })))
        .await;
    let Some(Frame::Response(answered)) = connection.frame().await else {
        panic!("no response")
    };
    f.stop().await.unwrap();

    assert!(matches!(&refused.result, Outcome::Error { error } if error.kind == FailureKind::Invalid));
    assert!(
        matches!(&foreign.result, Outcome::Error { error } if error.kind == FailureKind::Invalid && error.message.contains('9')),
        "{foreign:?}"
    );
    assert_eq!(answered.id.0, 4);
    assert!(matches!(
        answered.result,
        Outcome::Ok {
            reply: Reply::Teams { .. }
        }
    ));
}

#[tokio::test]
async fn a_request_over_the_limit_ends_its_connection_and_not_the_daemon() {
    let mut f = Fixture::new(&solo_state(), 3, "hang");
    f.start().await;
    let mut connection = RawConnection::open(&f.home).await;
    let long = format!("{}\n", "x".repeat(agent_core::protocol::MAX_REQUEST_BYTES + 1));

    connection.send(&long).await;
    let frame = connection.frame().await;

    assert!(
        matches!(&frame, Some(agent_core::protocol::Frame::Response(r)) if matches!(&r.result, agent_core::protocol::Outcome::Error { error } if error.kind == FailureKind::Invalid)),
        "{frame:?}"
    );
    assert!(connection.frame().await.is_none(), "the connection is closed");
    let mut fresh = f.connect().await;
    fresh
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    f.stop().await.unwrap();
}
