//! What a lead wrote during its run, found by the mark the run began at: the newest message of the bus then, and the
//! direct messages of the lead after it.

use super::*;

const TEAM: &str = "team-20261120-191109-jeuf";
const TASK: &str = "task-20261120-191109-qesn";

/// A directory with `lines` as the log of the bus.
fn bus(lines: &[String]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    dir
}

/// A message of `from` to `to`.
fn chat(id: u64, from: &str, to: &str, text: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"{from}","to":"{to}","kind":"chat","text":{text}}}"#)
}

/// What `lead` wrote to `claude-1` after the message `after`.
async fn said(dir: &Path, after: Option<u64>) -> Vec<(u64, String)> {
    let (agent, team, task): (AgentName, TeamId, TaskId) = (
        "claude-1".parse().unwrap(),
        TEAM.parse().unwrap(),
        TASK.parse().unwrap(),
    );
    said_after(
        dir,
        Reader {
            agent: &agent,
            team: &team,
            task: &task,
        },
        "lead",
        after.and_then(NonZeroU64::new),
    )
    .await
    .unwrap()
    .into_iter()
    .map(|message| (message.id.get(), message.text))
    .collect()
}

#[tokio::test]
async fn the_newest_message_of_the_bus_is_the_mark() {
    let dir = bus(&[
        chat(1, "lead", "claude-1", "a"),
        chat(2, "x", "y", "b"),
        chat(7, "lead", "all", "c"),
    ]);
    let empty = tempfile::tempdir().unwrap();

    assert_eq!(newest_message_id(dir.path()).await.unwrap(), NonZeroU64::new(7));
    assert_eq!(newest_message_id(empty.path()).await.unwrap(), None);
}

#[tokio::test]
async fn only_the_direct_messages_of_the_lead_after_the_mark_are_the_answer() {
    let long = "y".repeat(MAX_TEXT_CHARS + 50);
    let dir = bus(&[
        chat(1, "lead", "claude-1", "before the run"),
        chat(2, "lead", "all", "a broadcast"),
        chat(3, "lead", "claude-1", "fix the reader zażółć"),
        chat(4, "inv-r1", "claude-1", "another sender"),
        chat(5, "lead", "claude-2", "another member"),
        chat(6, "lead", "claude-1", &long),
    ]);

    assert_eq!(
        said(dir.path(), Some(1)).await,
        [(3, "fix the reader zażółć".to_owned()), (6, long)]
    );
    assert_eq!(said(dir.path(), Some(6)).await, Vec::new());
    assert_eq!(
        said(dir.path(), None).await.len(),
        3,
        "with no mark every direct message of the lead"
    );
}

#[tokio::test]
async fn an_empty_bus_has_no_answer_and_a_line_that_is_no_message_is_an_error() {
    let empty = tempfile::tempdir().unwrap();
    let broken = bus(&["not json".to_owned()]);

    assert_eq!(said(empty.path(), None).await, Vec::new());
    assert!(matches!(
        newest_message_id(broken.path()).await,
        Err(UnreadMessagesError::Line { line: 1, .. })
    ));
}
