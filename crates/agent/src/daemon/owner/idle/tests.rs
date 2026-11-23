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
