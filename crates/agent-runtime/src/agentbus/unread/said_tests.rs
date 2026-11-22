//! The newest message an agent wrote to a session, read or not.

use super::*;

const TEAM: &str = "team-20261120-191109-jeuf";
const TASK: &str = "task-20261120-191109-qesn";

/// A directory with `lines` as the log of the bus.
fn bus(lines: &[String]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    dir
}

/// A line of the log: `extra` is more JSON fields, with a leading comma.
fn line(id: u64, from: &str, to: &str, text: &str, extra: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"{from}","to":"{to}","text":{text}{extra}}}"#)
}

/// The newest message `sender` wrote to `claude-1` in the team of these tests.
async fn said(dir: &Path, sender: &str) -> Result<Option<UnreadMessage>, UnreadMessagesError> {
    let (agent, team, task): (AgentName, TeamId, TaskId) = (
        "claude-1".parse().unwrap(),
        TEAM.parse().unwrap(),
        TASK.parse().unwrap(),
    );
    last_message_from(
        dir,
        Reader {
            agent: &agent,
            team: &team,
            task: &task,
        },
        sender,
    )
    .await
}

fn message(id: u64, from: &str, text: &str) -> UnreadMessage {
    UnreadMessage {
        id: NonZeroU64::new(id).unwrap(),
        from: from.to_owned(),
        text: text.to_owned(),
    }
}

#[tokio::test]
async fn the_newest_message_of_the_sender_to_the_session_is_found_even_when_it_was_read() {
    let dir = bus(&[
        line(1, "lead", "claude-1", "fix the reader", ""),
        line(2, "agentbus", "claude-1", "notice", ""),
        line(3, "lead", "claude-1", "zażółć gęślą jaźń", ""),
        line(4, "lead", "claude-2", "for another agent", ""),
        line(5, "inv-r1", "claude-1", "another sender", ""),
    ]);

    assert_eq!(
        said(dir.path(), "lead").await.unwrap(),
        Some(message(3, "lead", "zażółć gęślą jaźń"))
    );
}

#[tokio::test]
async fn a_message_to_all_and_one_to_the_team_count_and_one_of_another_team_does_not() {
    let other = r#","team":"team-20261119-000000-zzzz""#;
    let dir = bus(&[
        line(1, "lead", "claude-1", "to the agent", ""),
        line(2, "lead", "all", "to all", ""),
        line(3, "lead", "claude-1", "bound to another team", other),
    ]);

    assert_eq!(
        said(dir.path(), "lead").await.unwrap(),
        Some(message(2, "lead", "to all"))
    );
}

#[tokio::test]
async fn a_sender_that_wrote_nothing_to_the_session_has_no_message() {
    let dir = bus(&[line(1, "inv-r1", "claude-1", "hello", "")]);

    assert_eq!(said(dir.path(), "lead").await.unwrap(), None);
}

#[tokio::test]
async fn an_empty_bus_directory_has_no_message() {
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(said(dir.path(), "lead").await.unwrap(), None);
}

#[tokio::test]
async fn a_line_that_is_no_message_is_an_error() {
    let dir = bus(&["not json".to_owned()]);

    assert!(matches!(
        said(dir.path(), "lead").await,
        Err(UnreadMessagesError::Line { line: 1, .. })
    ));
}
