//! What a lead wrote to a member after the notice that the member finished: the reason of a rejection, found by the
//! notice and not by the newest message of the lead.

use super::*;

const TEAM: &str = "team-20261120-191109-jeuf";
const TASK: &str = "task-20261120-191109-qesn";

/// A directory with `lines` as the log of the bus.
fn bus(lines: &[String]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    dir
}

/// A chat line of the log: `extra` is more JSON fields, with a leading comma.
fn chat(id: u64, from: &str, to: &str, text: &str, extra: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(
        r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"{from}","to":"{to}","kind":"chat","text":{text}{extra}}}"#
    )
}

/// The notice that tells `lead` that `member` finished item `item`.
fn finished_notice(id: u64, lead: &str, member: &str, item: &str) -> String {
    format!(
        r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"agentbus","to":"{lead}","kind":"script","text":"done","data":{{"notice":"member_finished","fields":{{"item":"{item}","member":"{member}","branch":"b"}},"body":"done"}}}}"#
    )
}

/// What `lead` wrote to `claude-1` after the newest notice that `claude-1` finished item 1.1.
async fn said(dir: &Path) -> Result<Option<Vec<Said>>, UnreadMessagesError> {
    let (agent, team, task): (AgentName, TeamId, TaskId) = (
        "claude-1".parse().unwrap(),
        TEAM.parse().unwrap(),
        TASK.parse().unwrap(),
    );
    said_since_finished(
        dir,
        Reader {
            agent: &agent,
            team: &team,
            task: &task,
        },
        "lead",
        Finished {
            lead: "lead",
            member: "claude-1",
            item: "1.1",
        },
    )
    .await
}

/// The ids and texts of `said`.
fn texts(said: Option<Vec<Said>>) -> Vec<(u64, String)> {
    said.unwrap_or_default()
        .into_iter()
        .map(|message| (message.id.get(), message.text))
        .collect()
}

#[tokio::test]
async fn only_the_direct_messages_of_the_lead_after_the_newest_notice_are_the_answer() {
    let dir = bus(&[
        chat(1, "lead", "claude-1", "an answer to the first hand-over", ""),
        finished_notice(2, "lead", "claude-1", "1.1"),
        chat(3, "lead", "all", "a broadcast", ""),
        chat(4, "lead", "claude-1", "fix the reader first", ""),
        chat(5, "inv-r1", "claude-1", "another sender", ""),
        chat(6, "lead", "team:team-20261120-191109-jeuf", "to the team", ""),
        chat(7, "lead", "claude-2", "to another member", ""),
        chat(8, "lead", "claude-1", "and add the tests zażółć", ""),
    ]);

    assert_eq!(
        texts(said(dir.path()).await.unwrap()),
        [
            (4, "fix the reader first".to_owned()),
            (8, "and add the tests zażółć".to_owned())
        ]
    );
}

#[tokio::test]
async fn a_second_notice_leaves_out_what_the_lead_said_after_the_first() {
    let dir = bus(&[
        finished_notice(1, "lead", "claude-1", "1.1"),
        chat(2, "lead", "claude-1", "the first return", ""),
        finished_notice(3, "lead", "claude-1", "1.1"),
        chat(4, "lead", "claude-1", "the second return", ""),
    ]);

    assert_eq!(
        texts(said(dir.path()).await.unwrap()),
        [(4, "the second return".to_owned())]
    );
}

#[tokio::test]
async fn a_notice_of_another_lead_member_or_item_is_not_the_notice() {
    let dir = bus(&[
        finished_notice(1, "other-lead", "claude-1", "1.1"),
        finished_notice(2, "lead", "claude-2", "1.1"),
        finished_notice(3, "lead", "claude-1", "1.2"),
        chat(4, "lead", "claude-1", "a message with no notice before it", ""),
    ]);

    assert_eq!(said(dir.path()).await.unwrap(), None);
}

#[tokio::test]
async fn a_notice_with_no_message_after_it_gives_an_empty_answer() {
    let dir = bus(&[finished_notice(1, "lead", "claude-1", "1.1")]);

    assert_eq!(said(dir.path()).await.unwrap(), Some(Vec::new()));
}

#[tokio::test]
async fn the_whole_text_of_a_long_message_is_kept() {
    let long = "x".repeat(MAX_TEXT_CHARS + 100);
    let dir = bus(&[
        finished_notice(1, "lead", "claude-1", "1.1"),
        chat(2, "lead", "claude-1", &long, ""),
    ]);

    assert_eq!(texts(said(dir.path()).await.unwrap()), [(2, long)]);
}

#[tokio::test]
async fn an_empty_bus_directory_has_no_notice_and_a_line_that_is_no_message_is_an_error() {
    let empty = tempfile::tempdir().unwrap();
    let broken = bus(&["not json".to_owned()]);

    assert_eq!(said(empty.path()).await.unwrap(), None);
    assert!(matches!(
        said(broken.path()).await,
        Err(UnreadMessagesError::Line { line: 1, .. })
    ));
}
