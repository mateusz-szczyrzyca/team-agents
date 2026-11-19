//! Integration tests for disk space monitoring, target cleanup and run refusals.

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;

use agent_core::config::AbsolutePath;
use agent_core::history::MaxTextBytes;
use agent_core::ids::AgentName;
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::protocol::{AgentList, Client, Command, FailureKind, RunRequest};
use agent_core::state::{ItemPhase, MergeMode, ReviewOutcome, TaskListText};
use agent_runtime::clock::SharedClock;
use agent_runtime::disk::{FreeSpace, FreeSpaceError, GIBIBYTE, SharedFreeSpace};
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, HistoryLayer, Problem, Reader, Reporter, Setup};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

use super::rig::{CHAIN, ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};
use crate::daemon::owner::Owner;
use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{repo, solo_state, task};
use agent_core::ids::TaskId;

/// A published lead and two approving reviewers.
async fn rig() -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig
}

/// Waits for a notification containing `needle`.
async fn wait_for_notification(rig: &Rig, needle: &str) -> String {
    rig.wait_for(needle, || rig.notifications().iter().any(|text| text.contains(needle)))
        .await;
    rig.notifications()
        .into_iter()
        .find(|text| text.contains(needle))
        .unwrap()
}

#[tokio::test]
async fn run_request_refused_below_min_free_space() {
    let mut rig = rig().await;
    // Set free space below min_free_gb (20 GB).
    rig.free_space.set(10 * 1024 * 1024 * 1024);
    rig.start().await;

    let names: Vec<AgentName> = vec!["claude-1".parse().unwrap()];
    let request = RunRequest {
        repo: "example".parse().unwrap(),
        checkout: AbsolutePath::from_str(rig.repo.to_str().unwrap()).unwrap(),
        agents: AgentList::try_from(names).unwrap(),
        task_list: TaskListText::new(ONE.to_owned()).unwrap(),
        coordinator: None,
        merge: MergeMode::Autopilot,
    };
    let result = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Run(request))
        .await;

    match result {
        Err(ClientError::Refused(failure)) => {
            assert_eq!(failure.kind, FailureKind::Refused);
            assert!(
                failure
                    .message
                    .contains("not enough free disk space: 10 GB free, minimum is 20 GB"),
                "unexpected message: {}",
                failure.message
            );
            assert!(
                failure
                    .message
                    .contains("remove build directories or unused worktrees in"),
                "unexpected message: {}",
                failure.message
            );
        }
        other => panic!("expected refused, got {other:?}"),
    }

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn low_disk_notifies_once_and_removes_reviewer_target_on_settle() {
    let mut rig = rig().await;
    // Set space below low_free_gb (100 GB) but above min_free_gb (20 GB).
    rig.free_space.set(50 * 1024 * 1024 * 1024);
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;

    // Wait until lead finishes and reviewers are active / approved.
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;

    // Check that low disk notification was delivered.
    let notice = wait_for_notification(&rig, "Low free disk space").await;
    assert!(notice.contains("Low free disk space"));
    assert!(notice.contains("50 GB free"));
    assert!(notice.contains("configured limit of 100 GB"));

    // Verify reviewer target removal on settle.
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    assert!(tree.exists());
    assert!(!tree.join("target").exists(), "reviewer target should not exist");

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn queued_tasks_wait_below_min_free_space_and_resume_when_space_back() {
    let mut rig = rig().await;
    rig.start().await;
    rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;

    // 1.1 finishes and merges.
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    // Drop free space below min_free_gb (20 GB).
    rig.free_space.set(10 * 1024 * 1024 * 1024);

    // Wait for the disk wait notification.
    let notice = wait_for_notification(&rig, "Rounds waiting for free disk space").await;
    assert!(notice.contains("Rounds waiting for free disk space"));
    assert!(notice.contains("10 GB free"));
    assert!(notice.contains("minimum threshold of 20 GB"));

    // Task 1.2 should not start yet; its task is queued waiting for disk space.
    // No wait is needed: while the space is below the minimum, the task cannot start.
    let state = rig.state();
    let task_1 = state.teams()[0].crew().roster()[0].1.task();
    assert_eq!(
        state.find_task(task_1).unwrap().member.status(),
        agent_core::status::TaskStatus::Queued,
        "task should be queued waiting for disk space"
    );

    // Restore space above min_free_gb.
    rig.free_space.set(200 * 1024 * 1024 * 1024);

    // 1.2 should now start and finish.
    rig.item_where("1.2", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    rig.stop().await.unwrap();
}

/// A reporter that says nothing: the test reads the row.
#[derive(Debug)]
struct SilentReporter;

impl Reporter for SilentReporter {
    fn report(&self, _problem: &Problem) {}
}

/// Removes a worktree's build directory and records `BuildDirectoryRemoved` in `observed` once its row is found at its level.
pub(super) async fn record_build_directory_removed(observed: &mut BTreeSet<LogEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let daemon = Daemon::new(&solo_state(), None);
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        agent_core::config::Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();

    let worktree_dir = daemon.home.worktree_dir(&repo(), &task(1));
    let target = worktree_dir.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("output.bin"), b"build output").unwrap();

    let (owner, _sinks) = daemon.bare_owner().await;
    let mut owner = owner.with_history(started.history.clone());
    async {
        owner.remove_target(&task(1), &worktree_dir);
        let joined = owner
            .disk_jobs
            .join_next_with_id()
            .await
            .expect("the removal was started");
        owner.disk_job_done(joined).await;
    }
    .with_subscriber(dispatch)
    .await;

    started.writer.close().unwrap();
    let logs = logs_in(&path);
    let removed = row(&logs, LogEvent::BuildDirectoryRemoved);
    assert_eq!(removed.event(), LogEvent::BuildDirectoryRemoved);
    assert_eq!(removed.level(), LogEvent::BuildDirectoryRemoved.level(), "{removed:?}");
    observed.insert(LogEvent::BuildDirectoryRemoved);
}

#[tokio::test]
async fn removing_a_build_directory_is_logged_as_build_directory_removed() {
    let mut observed = BTreeSet::new();
    record_build_directory_removed(&mut observed).await;
    assert!(observed.contains(&LogEvent::BuildDirectoryRemoved));
}

/// The rows of `daemon_logs` in the history file `path`.
fn logs_in(path: &Path) -> Vec<LogEntry> {
    Reader::open(path).unwrap().daemon_logs().unwrap()
}

/// The first row of `event` in `logs`; the test fails with every row when there is none.
fn row(logs: &[LogEntry], event: LogEvent) -> &LogEntry {
    logs.iter()
        .find(|entry| entry.event() == event)
        .unwrap_or_else(|| panic!("no {event} row in {logs:#?}"))
}

/// Two reviewers that build: each leaves a `target/` directory in its worktree before it approves.
async fn building_rig() -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "target-done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "target-done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig
}

/// The reviewer of the item that its run has approved, with its worktree.
async fn approved_reviewer(rig: &mut Rig) -> (agent_core::ids::TaskId, PathBuf) {
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    (reviewer, tree)
}

#[tokio::test]
async fn reviewer_target_is_removed_when_the_space_is_below_the_low_limit() {
    let mut rig = building_rig().await;
    rig.free_space.set(50 * 1024 * 1024 * 1024);
    rig.start().await;

    let (_reviewer, tree) = approved_reviewer(&mut rig).await;
    let target = tree.join("target");
    rig.wait_for("the reviewer target to be removed", || !target.exists())
        .await;

    assert!(tree.exists(), "the checkout stays");
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn low_disk_notice_is_sent_once_while_the_space_stays_low() {
    let mut rig = rig().await;
    rig.free_space.set(50 * 1024 * 1024 * 1024);
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    wait_for_notification(&rig, "Low free disk space").await;

    let notices = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("Low free disk space"))
        .count();
    assert_eq!(notices, 1, "one notice per crossing, team {team}");
    rig.stop().await.unwrap();
}

/// A round whose reviewers are one that approves at once and one that is held: the round stays open until the held one
/// reports, so no round close removes what the quick reviewer built.
async fn round_held_by_one_reviewer(free_bytes: u64) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "target-done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "held-review"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig.free_space.set(free_bytes);
    rig
}

/// The two reviewers of the team: the held one (its ready marker is there) and the quick one; waits for the held one.
async fn reviewers_of(rig: &Rig, team: &agent_core::ids::TeamId) -> (TaskId, TaskId) {
    let state = rig
        .state_where("the held reviewer to be ready", |state| {
            state.team(team).is_some_and(|team| {
                team.reviewers()
                    .iter()
                    .any(|member| rig.worktree(member.task()).join(".fake-review-ready-1").is_file())
            })
        })
        .await;
    let reviewers: Vec<TaskId> = state
        .team(team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| member.task().clone())
        .collect();
    let held = reviewers
        .iter()
        .find(|task| rig.worktree(task).join(".fake-review-ready-1").is_file())
        .unwrap()
        .clone();
    let quick = reviewers.iter().find(|task| **task != held).unwrap().clone();
    (held, quick)
}

#[tokio::test]
async fn reviewer_target_is_removed_at_the_end_of_its_run_below_the_low_limit() {
    let mut rig = round_held_by_one_reviewer(50 * 1024 * 1024 * 1024).await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let (held, quick) = reviewers_of(&rig, &team).await;
    let target = rig.worktree(&quick).join("target");

    rig.wait_for("the quick reviewer's target to be removed", || !target.exists())
        .await;

    let state = rig.state();
    assert!(
        matches!(
            state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap().phase(),
            ItemPhase::InReview { .. }
        ),
        "the round must still be open"
    );
    release_review(&rig, &held, "VERDICT: APPROVE\nSUMMARY: fine");
    rig.item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn reviewer_target_is_kept_at_the_end_of_its_run_above_the_low_limit() {
    let mut rig = round_held_by_one_reviewer(150 * 1024 * 1024 * 1024).await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let (held, quick) = reviewers_of(&rig, &team).await;
    let target = rig.worktree(&quick).join("target");
    rig.state_where("the quick reviewer's verdict to be recorded", |state| {
        matches!(
            state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap().phase(),
            ItemPhase::InReview { reviews, .. }
                if reviews.iter().any(|review| matches!(review.outcome, ReviewOutcome::Submitted { .. }))
        )
    })
    .await;

    assert!(target.join("output.bin").exists(), "the build stays above the limit");
    release_review(&rig, &held, "VERDICT: APPROVE\nSUMMARY: fine");
    rig.item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_run_that_works_is_not_stopped_when_the_space_falls_below_the_minimum() {
    let mut rig = round_held_by_one_reviewer(50 * 1024 * 1024 * 1024).await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let (held, _quick) = reviewers_of(&rig, &team).await;

    rig.free_space.set(10 * 1024 * 1024 * 1024);
    release_review(&rig, &held, "VERDICT: APPROVE\nSUMMARY: fine");

    rig.item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
}

/// Sends the report of the held reviewer `task` in its first pass.
fn release_review(rig: &Rig, task: &TaskId, report: &str) {
    std::fs::write(rig.worktree(task).join(".fake-review-result-1"), format!("{report}\n")).unwrap();
}

#[tokio::test]
async fn the_build_directory_of_a_lead_is_removed_when_its_round_closes() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "target-work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig.free_space.set(150 * 1024 * 1024 * 1024);
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    let lead = state.team(&team).unwrap().crew().roster()[0].1.task().clone();

    rig.wait_for("the lead's build directory to be removed", || {
        !rig.worktree(&lead).join("target").exists()
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_state_that_cannot_be_saved_runs_the_emergency_command_with_the_reason() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let emergency = daemon.home.root().join("emergency.txt");
    let script = format!("printf '%s' \"$1\" > '{}'", emergency.display());
    let text = format!(
        "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\n\n[notifications]\ncommand = [\"sh\", \"-c\", {script:?}, \"sh\"]\n"
    );
    let config = agent_core::config::Config::from_toml_str(&text).unwrap();
    owner.parts.notifier = agent_runtime::notify::Notifier::from_config(
        config.notifications(),
        &daemon.home,
        &agent_core::config::Secrets::default(),
    );
    // A directory where state.json is written makes the save fail: the rename onto it cannot succeed.
    std::fs::remove_file(daemon.home.state_file()).unwrap();
    std::fs::create_dir(daemon.home.state_file()).unwrap();

    let result = owner.save().await;

    assert!(
        matches!(result, Err(crate::daemon::owner::OwnerError::Store(_))),
        "{result:?}"
    );
    let reason = std::fs::read_to_string(&emergency).expect("the emergency command ran");
    assert!(reason.contains("state.json"), "{reason}");
}

#[tokio::test]
async fn agent_status_running_names_the_free_space_below_the_limit() {
    let mut rig = round_held_by_one_reviewer(50 * 1024 * 1024 * 1024).await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    reviewers_of(&rig, &team).await;

    let mut connection = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap();
    let activity = crate::commands::status::Activity::new(rig.home.clone(), time::UtcOffset::UTC);
    let mut out = Vec::new();
    crate::commands::status::status_running(&mut connection, &activity, &mut out)
        .await
        .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("disk: 50 GB free, limit 100 GB"), "{text}");
    rig.stop().await.unwrap();
}

/// Free space readings in the order the owner asks for them: each call takes the next reading, the last one stays.
#[derive(Debug)]
struct Readings(Mutex<VecDeque<u64>>);

impl FreeSpace for Readings {
    fn free_space_bytes(&self, _path: &Path) -> Result<u64, FreeSpaceError> {
        let mut values = self.0.lock().unwrap();
        if values.len() > 1 {
            Ok(values.pop_front().unwrap())
        } else {
            Ok(*values.front().unwrap())
        }
    }
}

/// A bare owner whose free space is read from `gibibytes`, and whose notifications are written to its home.
async fn owner_reading(gibibytes: &[u64]) -> (Daemon, Owner) {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let readings = Readings(Mutex::new(gibibytes.iter().map(|gb| gb * GIBIBYTE).collect()));
    owner.parts.free_space = SharedFreeSpace::new(readings);
    (daemon, owner)
}

/// Waits for every disk job to end and handles its outcome, as the owner's loop does.
async fn finish_disk_jobs(owner: &mut Owner) {
    while let Some(joined) = owner.disk_jobs.join_next_with_id().await {
        owner.disk_job_done(joined).await;
    }
}

/// How many notifications of the low-space notice the home holds.
fn low_space_notices(daemon: &Daemon) -> usize {
    let Ok(entries) = std::fs::read_dir(daemon.home.notifications_dir()) else {
        return 0;
    };
    entries
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .filter(|text| text.contains("Low free disk space"))
        .count()
}

#[tokio::test]
async fn low_space_is_notified_once_per_crossing_and_again_after_recovery() {
    // Below the low limit, still below it, above it again, and below it once more.
    let (daemon, mut owner) = owner_reading(&[50, 50, 150, 50]).await;

    owner.check_free_space();
    finish_disk_jobs(&mut owner).await;
    assert_eq!(low_space_notices(&daemon), 1, "the first crossing is notified");

    owner.check_free_space();
    finish_disk_jobs(&mut owner).await;
    assert_eq!(
        low_space_notices(&daemon),
        1,
        "staying below the limit is not notified again"
    );

    owner.check_free_space();
    finish_disk_jobs(&mut owner).await;
    assert_eq!(
        low_space_notices(&daemon),
        1,
        "recovering above the limit is not notified"
    );

    owner.check_free_space();
    finish_disk_jobs(&mut owner).await;
    assert_eq!(
        low_space_notices(&daemon),
        2,
        "a new crossing after recovery is notified"
    );
}

#[tokio::test]
async fn a_run_is_refused_when_the_space_falls_below_the_minimum_between_two_launches() {
    // The first reading (200 GB) lets the queued task start; the reading before its launch (10 GB) refuses it.
    let (_daemon, mut owner) = owner_reading(&[200, 10]).await;

    owner.start_queued().await.unwrap();

    assert_eq!(owner.runs.len(), 0, "no run started below the minimum");
    let member = &owner.parts.state.teams()[0].crew().roster()[0].1;
    assert_eq!(member.status(), agent_core::status::TaskStatus::Queued);
}

#[tokio::test]
async fn a_queued_task_is_not_started_while_its_build_directory_is_being_removed() {
    let (_daemon, mut owner) = owner_reading(&[200]).await;
    let member = owner.parts.state.teams()[0].crew().roster()[0].1.task().clone();
    // A removal of the task's directory that does not end until the test releases it.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let removed = member.clone();
    let removal = owner.disk_jobs.spawn_blocking(move || {
        held.recv().expect("the test releases the removal");
        crate::daemon::owner::disk::DiskJob::Target {
            task: removed,
            outcome: crate::daemon::owner::disk::BuildDirOutcome::Released { bytes: 0 },
        }
    });
    owner.pending_removals.start(removal.id(), &member);

    owner.start_queued().await.unwrap();

    assert_eq!(owner.runs.len(), 0, "the task waits for its removal");
    let status = owner.parts.state.teams()[0].crew().roster()[0].1.status();
    assert_eq!(
        status,
        agent_core::status::TaskStatus::Queued,
        "a launch would have failed the task"
    );
    release.send(()).unwrap();
    finish_disk_jobs(&mut owner).await;
    assert!(!owner.pending_removals.contains(&member));
}
