//! Contracts for supported log formats, tail limits and missing sources.
use super::*;
use agent_core::config::AbsolutePath;
use agent_core::state::{Crew, Member, MergeMode, Participant, TaskListText};

fn team() -> Team {
    Team::new(
        "team-20261117-120000-a1b2".parse().unwrap(),
        "example".parse().unwrap(),
        "2026-11-17T12:00:00Z".parse().unwrap(),
        MergeMode::NoMerge,
        TaskListText::new("Work\n".to_owned()).unwrap(),
        Crew::solo(Member::new(
            "task-20261117-120000-a1b2".parse().unwrap(),
            Participant::Normal("codex-1".parse().unwrap()),
        ))
        .unwrap(),
    )
}

#[tokio::test]
async fn distinguishes_missing_default_explicit_missing_and_empty_log() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    assert_eq!(collect(&home, None, &team()).await.unwrap(), None);
    let path = home.root().join("explicit.log");
    assert!(
        matches!(collect(&home, Some(&path), &team()).await, Err(LogError::Read {path: failed, source}) if failed == path && source.kind() == std::io::ErrorKind::NotFound)
    );
    std::fs::write(&path, "").unwrap();
    assert_eq!(collect(&home, Some(&path), &team()).await.unwrap(), Some(vec![]));
}

#[tokio::test]
async fn filters_json_levels_and_text_levels_from_only_the_last_500_lines() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let text = format!(
        "WARN old\n{}{}\n{}\n{}\n{}\n{}\n",
        "unrelated\n".repeat(495),
        r#"{"level":"INFO","message":"error mentioned in harmless text"}"#,
        r#"{"level":"WARN","message":"other team"}"#,
        "task-20261117-120000-a1b2 step",
        "ERROR another team",
        "unrelated final"
    );
    std::fs::write(home.root().join("daemon.log"), text).unwrap();
    assert_eq!(
        collect(&home, None, &team()).await.unwrap(),
        Some(vec![
            r#"{"level":"WARN","message":"other team"}"#.to_owned(),
            "task-20261117-120000-a1b2 step".to_owned(),
            "ERROR another team".to_owned()
        ])
    );
}

#[tokio::test]
async fn rejects_invalid_text_instead_of_losing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    std::fs::write(home.root().join("daemon.log"), [0xff]).unwrap();
    assert!(matches!(collect(&home, None, &team()).await, Err(LogError::Utf8(_))));
}

#[tokio::test]
async fn review_daemon_keeps_team_lines_before_the_recent_warning_window() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team_line = "team-20261117-120000-a1b2 started";
    let text = format!("{team_line}\nWARN old\n{}WARN recent\n", "unrelated\n".repeat(500));
    std::fs::write(home.root().join("daemon.log"), text).unwrap();

    assert_eq!(
        collect(&home, None, &team()).await.unwrap(),
        Some(vec![team_line.to_owned(), "WARN recent".to_owned()])
    );
}

#[tokio::test]
async fn review_daemon_reads_only_the_snapshot_window_when_the_log_grows() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.log");
    let body = "x\n".repeat(2 * 1024 * 1024 + 1);
    std::fs::write(&path, &body).unwrap();
    let len = std::fs::metadata(&path).unwrap().len();
    let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    writer.write_all(b"grown\n").unwrap();
    let file = tokio::fs::File::open(&path).await.unwrap();

    let bytes = read_tail(file, len, &path).await.unwrap();

    assert_eq!(
        (bytes.len(), bytes.as_chunks::<2>().0.iter().all(|pair| pair == b"x\n")),
        (4 * 1024 * 1024, true)
    );
}
