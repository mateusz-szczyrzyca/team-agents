//! Removal through the socket against the existing fake GitHub and agent CLI.

use agent_core::protocol::{Client, Command, ControlRequest, RemoveRequest, Reply, StatusRequest, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::start::work_branch;

#[tokio::test]
async fn removal_notifies_once_closes_the_pull_and_survives_restart_without_losing_history_or_remote_work() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "clean-sleep"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        0,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let task = tasks.first().unwrap();
    rig.status_is(task, TaskStatus::Waiting).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Team(team.clone()),
            }),
        )
        .await
        .unwrap();
    rig.stop().await.unwrap();
    let before = Reader::open(&rig.home.root().join("history.db"))
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert!(!before.is_empty());
    let state = rig.state();
    let found = state.find_task(task).unwrap();
    let branch = work_branch(found.team, found.member).unwrap();
    let worktree = rig.worktree(task);
    std::fs::write(worktree.join("kept.txt"), "unmerged work\n").unwrap();
    git(&worktree, &["add", "kept.txt"]);
    git(&worktree, &["commit", "-q", "-m", "Keep this work"]);
    git(&worktree, &["push", "-q", "origin", branch.as_str()]);
    let remote = rig.remote_tip(branch.as_str());
    let pull = rig.github.foreign_pull(branch.as_str());
    let older_notifications = rig.notifications();
    rig.start().await;
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Remove(RemoveRequest {
                team: team.clone(),
                force: false,
            }),
        )
        .await
        .unwrap();
    let Reply::Removed { removal } = reply else {
        panic!("expected removal, got {reply:?}")
    };
    assert_eq!(
        removal
            .closed_pull_requests
            .iter()
            .map(|pr| pr.get().get())
            .collect::<Vec<_>>(),
        vec![pull]
    );
    assert!(rig.github.pulls().iter().find(|pr| pr.number == pull).unwrap().closed);
    assert!(!rig.github.pulls().iter().find(|pr| pr.number == pull).unwrap().merged);
    let comments = rig.github.comments(pull);
    assert_eq!(comments.len(), 1);
    assert!(comments[0].contains(&team.to_string()));
    assert!(comments[0].contains("removed"));
    assert_eq!(rig.remote_tip(branch.as_str()), remote);
    let bundle = removal.bundle.as_ref().unwrap();
    git(
        &rig.repo,
        &["fetch", bundle.as_path().to_str().unwrap(), branch.as_str()],
    );
    assert_eq!(git(&rig.repo, &["rev-parse", "FETCH_HEAD"]), remote);
    assert!(!worktree.exists());
    assert!(!rig.home.task_dir(task).exists());
    let notices = rig.notifications();
    assert!(older_notifications.iter().all(|old| notices.contains(old)));
    let removal_notices: Vec<_> = notices.iter().filter(|text| text.contains("Team removed")).collect();
    assert_eq!(removal_notices.len(), 1);
    let notice = removal_notices[0];
    for text in [
        team.to_string(),
        "requested by the human".to_owned(),
        bundle.as_path().display().to_string(),
        removal.freed_bytes.to_string(),
    ] {
        assert!(notice.contains(&text), "missing {text:?} in {notice:?}");
    }
    for restart in [false, true] {
        if restart {
            rig.stop().await.unwrap();
            rig.start().await;
        }
        let status = Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
        assert_eq!(status, Reply::Teams { teams: vec![] });
        assert!(rig.state().find_task(task).is_none());
    }
    rig.stop().await.unwrap();
    let after = Reader::open(&rig.home.root().join("history.db"))
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert_eq!(after, before);
}

/// Proves the removal event through the owner, for the complete event-cause inventory too.
pub(super) async fn record_team_removed(observed: &mut std::collections::BTreeSet<agent_core::log::LogEvent>) {
    use std::sync::Arc;

    use agent_core::config::Secrets;
    use agent_core::history::MaxTextBytes;
    use agent_core::log::LogLevel;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, HistoryLayer, Problem, Reporter, Setup};
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;

    use crate::daemon::owner::fixture::Daemon;
    use crate::fixtures::{repo, solo_state, task, team_id};

    /// A reporter for this isolated history writer, whose failure must fail the test.
    #[derive(Debug)]
    struct TestReporter;
    impl Reporter for TestReporter {
        fn report(&self, problem: &Problem) {
            panic!("history failure: {problem:?}");
        }
    }

    let daemon = Daemon::new(&solo_state(), None);
    let path = daemon.home.root().join("removal-history.db");
    let started = History::start(
        HistoryDb::open(&path, MaxTextBytes::default()).unwrap(),
        Setup {
            reporter: Arc::new(TestReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();
    std::fs::create_dir_all(daemon.home.worktree_dir(&repo(), &task(1))).unwrap();
    let task_dir = daemon.home.task_dir(&task(1));
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::write(task_dir.join("notes.txt"), "history stays").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let removal = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .with_subscriber(dispatch)
        .await
        .unwrap()
        .unwrap();
    started.writer.close().unwrap();
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let matching: Vec<_> = rows
        .iter()
        .filter(|row| row.event().as_str() == "team_removed")
        .collect();
    assert_eq!(matching.len(), 1, "removal must have its own event: {rows:?}");
    let row = matching[0];
    assert_eq!(row.level(), LogLevel::Info);
    assert_eq!(row.team(), Some(&team_id(0)));
    assert_eq!(
        row.fields().get("reason").and_then(|v| v.as_str()),
        Some("requested by the human")
    );
    assert_eq!(
        row.fields().get("freed_bytes").and_then(|v| v.as_u64()),
        Some(removal.freed_bytes)
    );
    assert_eq!(row.fields().get("tasks").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(row.fields().get("bundle").and_then(|v| v.as_str()), Some("none"));
    observed.insert(row.event());
}

#[tokio::test]
async fn removal_records_its_own_event_with_reason_team_bundle_and_bytes() {
    let mut observed = std::collections::BTreeSet::new();
    record_team_removed(&mut observed).await;
    assert_eq!(observed.len(), 1);
}
