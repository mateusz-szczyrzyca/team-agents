//! Idle removal with a temporary home and the daemon's fake dependencies.
use super::super::fixture::{Daemon, pair_state};
use crate::fixtures::{at, repo, task, team_id};
use agent_core::status::TaskStatus;

/// Plain worktrees exercise document/art teams without requiring a Git repository.
fn files(daemon: &Daemon) {
    for n in [1, 2] {
        let path = daemon.home.worktree_dir(&repo(), &task(n));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("draft.txt"), "work").unwrap();
    }
}

#[tokio::test]
async fn old_state_is_removed_at_the_bound_and_not_given_a_new_startup_window() {
    let daemon = Daemon::new(&pair_state(), None);
    files(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(7199));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert_eq!(
        owner
            .parts
            .state
            .team(&team_id(0))
            .unwrap()
            .idle_watch()
            .map(|watch| watch.last_activity),
        Some(at(0))
    );
    daemon.clock.advance(time::Duration::seconds(1));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 0);
    assert_eq!(daemon.state().teams().len(), 0);
    for n in [1, 2] {
        assert!(!daemon.home.worktree_dir(&repo(), &task(n)).exists());
    }
    let notices = daemon.notifications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("idle"));
    assert!(notices[0].contains("2026-11-11"));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(daemon.notifications(), notices);
}

#[tokio::test]
async fn disabled_running_human_and_provider_holds_preserve_files() {
    for hold in ["disabled", "running", "paused", "stopped", "provider"] {
        let mut state = pair_state();
        if hold == "paused" || hold == "stopped" {
            state
                .find_task_mut(&task(1))
                .unwrap()
                .1
                .transition(if hold == "paused" {
                    TaskStatus::Paused
                } else {
                    TaskStatus::Stopped
                })
                .unwrap();
        }
        let daemon = Daemon::new(&state, None);
        if hold == "disabled" {
            daemon.configure("[limits]\nidle_team_minutes = 0\n");
        }
        files(&daemon);
        let (mut owner, _sinks) = daemon.bare_owner().await;
        if hold == "running" {
            owner.in_flight.insert(task(1));
        }
        if hold == "provider" {
            owner.wakes.insert(task(1), at(8000));
        }
        daemon.clock.advance(time::Duration::seconds(7200));
        owner.check_idle_teams().await.unwrap();
        assert_eq!(owner.parts.state.teams().len(), 1, "{hold}");
        assert!(daemon.home.worktree_dir(&repo(), &task(1)).exists(), "{hold}");
        assert_eq!(daemon.notifications(), Vec::<String>::new(), "{hold}");
    }
}

#[tokio::test]
async fn refusal_retains_team_and_notifies_once_across_restart() {
    let daemon = Daemon::new(&pair_state(), None);
    daemon.clock.advance(time::Duration::seconds(7200));
    let (mut owner, sinks) = daemon.bare_owner().await;
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert!(
        owner
            .parts
            .state
            .team(&team_id(0))
            .unwrap()
            .idle_watch()
            .is_some_and(|watch| watch.refusal.is_some())
    );
    let notices = daemon.notifications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("branch"));
    owner.check_idle_teams().await.unwrap();
    drop(owner);
    drop(sinks);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert_eq!(daemon.notifications(), notices);
}

/// Executes Git only in a disposable fixture, with deterministic commit dates.
fn git(path: &std::path::Path, seconds: u32, args: &[&str]) {
    let result = std::process::Command::new("git")
        .current_dir(path)
        .env("GIT_AUTHOR_DATE", at(seconds).to_string())
        .env("GIT_COMMITTER_DATE", at(seconds).to_string())
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8(result.stderr).unwrap());
}

#[tokio::test]
async fn git_dates_survive_startup_and_changed_tips_reset_activity() {
    let daemon = Daemon::new(&pair_state(), None);
    files(&daemon);
    let path = daemon.home.worktree_dir(&repo(), &task(1));
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), 7000, &["init", "-q", "-b", "main"]);
    git(
        repository.path(),
        7000,
        &["commit", "--allow-empty", "-qm", "Recent work"],
    );
    std::fs::remove_dir_all(&path).unwrap();
    let initial = format!("agents/current/{}", task(1));
    git(
        repository.path(),
        7000,
        &["worktree", "add", "-qb", &initial, path.to_str().unwrap()],
    );
    let (mut owner, sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(7200));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert_eq!(
        owner
            .parts
            .state
            .team(&team_id(0))
            .unwrap()
            .idle_watch()
            .unwrap()
            .last_activity,
        at(7000)
    );
    // A new branch containing an old-dated commit still counts when first observed.
    let branch = format!("agents/old/{}", task(2));
    git(&path, 0, &["checkout", "-qb", &branch]);
    git(&path, 0, &["commit", "--allow-empty", "-qm", "Imported work"]);
    git(&path, 0, &["checkout", "-q", &initial]);
    daemon.clock.advance(time::Duration::seconds(7000));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert_eq!(
        owner
            .parts
            .state
            .team(&team_id(0))
            .unwrap()
            .idle_watch()
            .unwrap()
            .last_activity,
        at(14200)
    );
    drop(owner);
    drop(sinks);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(7199));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    daemon.clock.advance(time::Duration::seconds(1));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 0);
    assert!(!path.exists());
}

#[tokio::test]
async fn damaged_git_metadata_refuses_removal_and_reports_once() {
    let daemon = Daemon::new(&pair_state(), None);
    files(&daemon);
    let path = daemon.home.worktree_dir(&repo(), &task(1));
    std::fs::write(path.join(".git"), "gitdir: /does-not-exist-fixture\n").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(7200));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert!(path.exists());
    assert_eq!(daemon.notifications().len(), 1);
    assert!(
        owner
            .parts
            .state
            .team(&team_id(0))
            .unwrap()
            .idle_watch()
            .unwrap()
            .heads
            .is_empty()
    );
    owner.check_idle_teams().await.unwrap();
    assert_eq!(daemon.notifications().len(), 1);
}

#[tokio::test]
async fn persisted_quota_hold_is_honored_until_its_recorded_reset() {
    let mut state = pair_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.begin_run(crate::fixtures::run(1), at(0), None).unwrap();
        member
            .end_run(
                at(1),
                agent_core::state::RunEnd::QuotaExhausted {
                    resets_at: Some(at(8000)),
                },
            )
            .unwrap();
        member.transition(TaskStatus::Waiting).unwrap();
    }
    let daemon = Daemon::new(&state, None);
    files(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(8059));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    daemon.clock.advance(time::Duration::seconds(1));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 0);
}

#[tokio::test]
async fn accepted_external_delivery_preserves_an_idle_team() {
    use agent_core::ids::RoundId;
    use agent_core::state::ItemPhase;
    let mut state = crate::fixtures::solo_state();
    let id = team_id(0);
    let round = RoundId::derive(&id, 1);
    let item = "1.1".parse().unwrap();
    {
        let mut team = state.team_mut(&id).unwrap();
        team.open_round(round.clone(), Some(item), at(1)).unwrap();
        team.advance_item(
            item,
            ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task".parse().unwrap(),
            },
        )
        .unwrap();
        team.activate_split(
            agent_core::state::split::SplitPlan::new(
                item,
                ["First", "Second"]
                    .into_iter()
                    .map(|title| agent_core::state::split::PartDefinition {
                        title: title.into(),
                        delivers: "A useful document".into(),
                        checks: "Read it".into(),
                        depends_on: Vec::new(),
                    })
                    .collect(),
            )
            .unwrap(),
        )
        .unwrap();
        team.deliver_round_with_pr(&round, crate::fixtures::pr(99)).unwrap();
    }
    let daemon = Daemon::new(&state, None);
    files(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(7200));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert!(daemon.home.worktree_dir(&repo(), &task(1)).exists());
    assert_eq!(daemon.notifications(), Vec::<String>::new());
}

#[tokio::test]
async fn idle_history_records_activity_and_refusal_once() {
    use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Reader, Setup, TracingReporter};
    use std::sync::Arc;
    for removable in [true, false] {
        let daemon = Daemon::new(&pair_state(), None);
        if removable {
            files(&daemon);
        }
        let started = History::start(
            HistoryDb::open(
                &daemon.home.history_file(),
                agent_core::history::MaxTextBytes::default(),
            )
            .unwrap(),
            Setup {
                capacity: DEFAULT_QUEUE,
                reporter: Arc::new(TracingReporter),
            },
        )
        .unwrap();
        let (mut owner, _sinks) = daemon.bare_owner().await;
        owner.history = started.history.clone();
        daemon.clock.advance(time::Duration::seconds(7200));
        owner.check_idle_teams().await.unwrap();
        owner.check_idle_teams().await.unwrap();
        let records = Reader::open(&daemon.home.history_file())
            .unwrap()
            .daemon_logs()
            .unwrap();
        let decisions: Vec<_> = records
            .iter()
            .filter(|entry| {
                entry
                    .fields()
                    .get("decision")
                    .is_some_and(|value| value == "idle_removal")
            })
            .collect();
        assert_eq!(decisions.len(), if removable { 1 } else { 2 });
        assert_eq!(decisions[0].fields()["last_activity"], at(0).to_string());
        assert_eq!(decisions[0].fields()["outcome"], "attempt");
        assert_eq!(decisions[0].fields()["items"], serde_json::json!([]));
        assert_eq!(decisions[0].fields()["rounds"], serde_json::json!([]));
        assert!(decisions[0].fields()["branches"].is_array());
        if !removable {
            assert_eq!(decisions[1].fields()["outcome"], "refused");
            assert_eq!(daemon.notifications().len(), 1);
        }
        started.writer.close().unwrap();
    }
}

#[tokio::test]
async fn unavailable_history_refuses_idle_deletion_without_losing_files() {
    let daemon = Daemon::new(&pair_state(), None);
    files(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.history = agent_runtime::history::History::unavailable(
        &std::io::Error::other("fixture history unavailable"),
        std::sync::Arc::new(agent_runtime::history::TracingReporter),
    );
    daemon.clock.advance(time::Duration::seconds(7200));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert!(daemon.home.worktree_dir(&repo(), &task(1)).exists());
    assert_eq!(daemon.notifications().len(), 1);
    assert!(daemon.notifications()[0].contains("history"));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(daemon.notifications().len(), 1);
}

#[tokio::test]
async fn reviewer_worktree_commits_keep_the_team_alive() {
    let daemon = Daemon::new(&super::super::fixture::reviewed_pair_state(), None);
    files(&daemon);
    let path = daemon.home.worktree_dir(&repo(), &task(3));
    std::fs::create_dir_all(&path).unwrap();
    git(&path, 7100, &["init", "-q", "-b", "main"]);
    git(&path, 7100, &["commit", "--allow-empty", "-qm", "Review work"]);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::seconds(7200));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(
        owner
            .parts
            .state
            .team(&team_id(0))
            .and_then(agent_core::state::Team::idle_watch)
            .map(|watch| watch.last_activity),
        Some(at(7100))
    );
    assert_eq!(daemon.notifications(), Vec::<String>::new());
}

#[path = "integration_tests.rs"]
mod integration;

#[tokio::test]
async fn provider_retry_queued_after_reset_survives_idle_scan_and_restart() {
    for end in [
        agent_core::state::RunEnd::QuotaExhausted {
            resets_at: Some(at(8000)),
        },
        agent_core::state::RunEnd::RateLimited,
    ] {
        let mut state = pair_state();
        {
            let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
            member.transition(TaskStatus::Running).unwrap();
            member.begin_run(crate::fixtures::run(1), at(0), None).unwrap();
            member.end_run(at(1), end).unwrap();
            member.transition(TaskStatus::Paused).unwrap();
        }
        let daemon = Daemon::new(&state, None);
        files(&daemon);
        let (mut owner, sinks) = daemon.bare_owner().await;
        owner.wakes.insert(task(1), at(8060));
        daemon.clock.advance(time::Duration::seconds(14400));
        owner.wake_due().await.unwrap();
        assert_eq!(
            owner.parts.state.find_task(&task(1)).unwrap().member.status(),
            TaskStatus::Queued
        );
        owner.check_idle_teams().await.unwrap();
        assert_eq!(owner.parts.state.teams().len(), 1);
        drop(owner);
        drop(sinks);
        let (mut owner, _sinks) = daemon.bare_owner().await;
        owner.check_idle_teams().await.unwrap();
        assert_eq!(owner.parts.state.teams().len(), 1);
        assert!(daemon.home.worktree_dir(&repo(), &task(1)).exists());
        assert_eq!(daemon.notifications(), Vec::<String>::new());
    }
}
