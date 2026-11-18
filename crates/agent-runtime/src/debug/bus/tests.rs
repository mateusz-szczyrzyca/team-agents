//! Unit tests for bus collection.

use super::*;
use crate::store::Home;
use agent_core::config::AbsolutePath;
use agent_core::state::{Crew, Member, MergeMode, Participant, TaskListText, Team};

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
async fn collects_team_messages_and_end_of_whole_bus() {
    let (_dir, home, team) = fixture();
    let bus_dir = home.root().join("bus");
    std::fs::create_dir_all(&bus_dir).unwrap();

    let mut lines = Vec::new();
    // 60 old unrelated messages (older than RECENT_MESSAGES threshold of 50)
    for i in 1..=60 {
        lines.push(format!(
            r#"{{"id":{i},"ts":"2026-11-17 09:00:00","from":"other-1","to":"other-2","kind":"chat","text":"old other message {i}"}}"#
        ));
    }
    // Old message about this team (must be kept even though > 50 messages old)
    lines.push(r#"{"id":61,"ts":"2026-11-17 10:05:00","from":"agentbus","to":"team:team-20261117-120000-a1b2","kind":"team","text":"team created"}"#.to_string());
    // Message from agent of this team
    lines.push(
        r#"{"id":62,"ts":"2026-11-17 10:10:00","from":"codex-1","to":"all","kind":"chat","text":"hello from codex"}"#
            .to_string(),
    );
    // Message mentioning task
    lines.push(r#"{"id":63,"ts":"2026-11-17 10:15:00","from":"agentbus","to":"all","kind":"notice","text":"task-20261117-120000-a1b2 started"}"#.to_string());
    // Recent message from another team (within recent limit of 50)
    lines.push(r#"{"id":64,"ts":"2026-11-17 11:00:00","from":"unrelated","to":"all","kind":"chat","text":"recent unrelated message"}"#.to_string());

    std::fs::write(bus_dir.join("messages.jsonl"), lines.join("\n") + "\n").unwrap();

    let collected = collect(&home, &team).await.unwrap().expect("bus messages");
    assert!(
        !collected
            .iter()
            .any(|m| m.contains("old other message 1)") || m.contains("old other message 1 ")),
        "old other message 1 should be excluded: {collected:?}"
    );
    assert!(
        collected.iter().any(|m| m.contains("team:team-20261117-120000-a1b2")),
        "team message kept"
    );
    assert!(
        collected
            .iter()
            .any(|m| m.contains("codex-1 → all (chat): hello from codex")),
        "agent message kept"
    );
    assert!(
        collected
            .iter()
            .any(|m| m.contains("task-20261117-120000-a1b2 started")),
        "task message kept"
    );
    assert!(
        collected.iter().any(|m| m.contains("recent unrelated message")),
        "recent message kept"
    );
}

#[tokio::test]
async fn collects_empty_when_bus_file_missing() {
    let (_dir, home, team) = fixture();
    let collected = collect(&home, &team).await.unwrap();
    assert_eq!(collected, Some(Vec::new()));
}

#[tokio::test]
async fn reports_unreadable_when_bus_file_is_directory() {
    let (_dir, home, team) = fixture();
    let bus_dir = home.root().join("bus");
    std::fs::create_dir_all(bus_dir.join("messages.jsonl")).unwrap();
    let result = collect(&home, &team).await;
    assert!(result.is_err(), "directory must trigger read error");
}
