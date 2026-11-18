//! Unit tests for notification collection.

use agent_core::config::AbsolutePath;
use agent_core::state::{Crew, Member, MergeMode, Participant, TaskListText, Team};

use super::*;
use crate::store::Home;

fn fixture() -> (tempfile::TempDir, Home, Team) {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);
    let task = "task-20261117-120000-a1b2".parse().unwrap();
    let team_id = "team-20261117-120000-a1b2".parse().unwrap();
    let member = Member::new(task, Participant::Normal("codex-1".parse().unwrap()));
    let team = Team::new(
        team_id,
        "example".parse().unwrap(),
        "2026-11-17T12:00:00Z".parse().unwrap(),
        MergeMode::Autopilot,
        TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Work\n".to_owned()).unwrap(),
        Crew::solo(member).unwrap(),
    );
    (dir, home, team)
}

#[tokio::test]
async fn collects_team_notifications_whole_newest_last_and_ignores_other_teams() {
    let (_dir, home, team) = fixture();
    let notes_dir = home.notifications_dir();
    std::fs::create_dir_all(&notes_dir).unwrap();

    let note_team_old = "Older Notification\nRepo: example\nTask: task-20261117-120000-a1b2\nNote: questions\n";
    let note_other = "Other Team\nRepo: example\nTask: task-other-9999\nNote: other team note\n";
    let note_team_new = "Newer Notification\nRepo: example\nTask: task-20261117-120000-a1b2\nNote: done\n";

    std::fs::write(notes_dir.join("notification-20261117100000-1.md"), note_team_old).unwrap();
    std::fs::write(notes_dir.join("notification-20261117103000-1.md"), note_other).unwrap();
    std::fs::write(notes_dir.join("notification-20261117110000-1.md"), note_team_new).unwrap();

    let collected = collect(&home, &team).await.unwrap().expect("notifications");
    assert_eq!(collected.len(), 2, "only 2 team notifications: {collected:?}");
    assert!(collected[0].contains("Older Notification"), "{}", collected[0]);
    assert!(collected[0].contains("Note: questions"), "{}", collected[0]);
    assert!(collected[1].contains("Newer Notification"), "{}", collected[1]);
    assert!(collected[1].contains("Note: done"), "{}", collected[1]);
}

#[tokio::test]
async fn collects_empty_when_notifications_directory_missing() {
    let (_dir, home, team) = fixture();
    let collected = collect(&home, &team).await.unwrap();
    assert_eq!(collected, Some(Vec::new()));
}

#[tokio::test]
async fn reports_unreadable_when_notifications_is_a_file() {
    let (_dir, home, team) = fixture();
    std::fs::write(home.notifications_dir(), b"not a directory\n").unwrap();
    let result = collect(&home, &team).await;
    assert!(result.is_err(), "must report read error when dir is a file");
}
