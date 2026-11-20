use super::*;

const TEAM: &str = "team-20261120-191109-jeuf";
const OTHER_TEAM: &str = "team-20261119-000000-zzzz";
const TASK: &str = "task-20261120-191109-qesn";
const OTHER_TASK: &str = "task-20261119-000000-abcd";

fn agent() -> AgentName {
    "deepseek-1".parse().unwrap()
}

fn team() -> TeamId {
    TEAM.parse().unwrap()
}

fn task() -> TaskId {
    TASK.parse().unwrap()
}

/// A line of the log: `extra` is more JSON fields, with a leading comma.
fn line(id: u64, from: &str, to: &str, text: &str, extra: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"{from}","to":"{to}","text":{text}{extra}}}"#)
}

/// A bus directory with the lines as its log and `cursors` (JSON) in its state, or no state at all.
fn bus(lines: &[String], cursors: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    if let Some(cursors) = cursors {
        let state = format!(r#"{{"version":1,"agents":{{}},"cursors":{cursors},"teams":{{}}}}"#);
        std::fs::write(dir.path().join("state.json"), state).unwrap();
    }
    dir
}

async fn unread(dir: &Path) -> Result<UnreadMessages, UnreadMessagesError> {
    let (agent, team, task) = (agent(), team(), task());
    unread_messages(
        dir,
        Reader {
            agent: &agent,
            team: &team,
            task: &task,
        },
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

fn ids(found: &UnreadMessages) -> Vec<u64> {
    found.shown.iter().map(|message| message.id.get()).collect()
}

#[tokio::test]
async fn an_empty_bus_directory_has_no_unread_message() {
    let dir = tempfile::tempdir().unwrap();

    let found = unread(dir.path()).await.unwrap();

    assert_eq!(
        found,
        UnreadMessages {
            shown: Vec::new(),
            completeness: Completeness::All,
        }
    );
}

#[tokio::test]
async fn the_messages_after_the_cursor_of_the_agent_in_its_team_are_unread() {
    let cursors = format!(r#"{{"deepseek-1/{TEAM}":2,"deepseek-1":5}}"#);
    let dir = bus(
        &[
            line(1, "inv-opus", "deepseek-1", "old", ""),
            line(2, "inv-opus", "deepseek-1", "read", ""),
            line(3, "inv-opus", "deepseek-1", "zażółć gęślą jaźń", ""),
        ],
        Some(&cursors),
    );

    let found = unread(dir.path()).await.unwrap();

    assert_eq!(
        found,
        UnreadMessages {
            shown: vec![message(3, "inv-opus", "zażółć gęślą jaźń")],
            completeness: Completeness::All,
        }
    );
}

#[tokio::test]
async fn a_team_without_a_cursor_of_its_own_reads_by_the_agents_single_cursor() {
    let dir = bus(
        &[
            line(1, "inv-opus", "deepseek-1", "read", ""),
            line(2, "inv-opus", "deepseek-1", "unread", ""),
        ],
        Some(r#"{"deepseek-1":1}"#),
    );

    assert_eq!(ids(&unread(dir.path()).await.unwrap()), vec![2]);
}

#[tokio::test]
async fn a_bus_without_a_state_has_every_message_for_the_session_unread() {
    let dir = bus(&[line(1, "inv-opus", "deepseek-1", "hello", "")], None);

    assert_eq!(ids(&unread(dir.path()).await.unwrap()), vec![1]);
}

#[tokio::test]
async fn only_the_messages_for_the_session_of_the_task_in_its_team_are_unread() {
    let lines = [
        line(1, "inv-opus", "deepseek-1", "direct, by name", ""),
        line(
            2,
            "inv-opus",
            "deepseek-1",
            "direct, in the team",
            &format!(r#","team":"{TEAM}""#),
        ),
        line(
            3,
            "inv-opus",
            "deepseek-1",
            "direct, another team",
            &format!(r#","team":"{OTHER_TEAM}""#),
        ),
        line(
            4,
            "inv-opus",
            "deepseek-1",
            "for the task",
            &format!(r#","team":"{TEAM}","task":"{TASK}""#),
        ),
        line(
            5,
            "inv-opus",
            "deepseek-1",
            "another task",
            &format!(r#","team":"{TEAM}","task":"{OTHER_TASK}""#),
        ),
        line(6, "human", TASK, "to the task by its id", ""),
        line(7, "inv-opus", &format!("team:{TEAM}"), "to the team", ""),
        line(8, "inv-opus", &format!("team:{OTHER_TEAM}"), "to another team", ""),
        line(9, "agentbus", "all", "to everybody", ""),
        line(10, "deepseek-1", "inv-opus", "written by the agent", ""),
        line(11, "inv-opus", "claude-1", "to somebody else", ""),
        line(12, "inv-opus", "deepseek-1", "quiet", r#","quiet":true"#),
    ];
    let dir = bus(&lines, Some("{}"));

    assert_eq!(ids(&unread(dir.path()).await.unwrap()), vec![1, 2, 4, 6, 7, 9, 12]);
}

#[tokio::test]
async fn more_unread_messages_than_shown_keep_the_newest() {
    let lines: Vec<String> = (1..=25)
        .map(|id| line(id, "inv-opus", "deepseek-1", &format!("message {id}"), ""))
        .collect();
    let dir = bus(&lines, Some("{}"));

    let found = unread(dir.path()).await.unwrap();

    assert_eq!(ids(&found), (6..=25).collect::<Vec<_>>());
    assert_eq!(found.completeness, Completeness::Newest);
}

#[tokio::test]
async fn exactly_as_many_unread_messages_as_shown_are_all_of_them() {
    let lines: Vec<String> = (1..=20)
        .map(|id| line(id, "inv-opus", "deepseek-1", "same", ""))
        .collect();
    let dir = bus(&lines, Some("{}"));

    let found = unread(dir.path()).await.unwrap();

    assert_eq!(found.shown.len(), MAX_MESSAGES);
    assert_eq!(found.completeness, Completeness::All);
}

#[tokio::test]
async fn a_long_message_is_cut_at_a_character() {
    let long = "ż".repeat(MAX_TEXT_CHARS + 10);
    let dir = bus(&[line(1, "inv-opus", "deepseek-1", &long, "")], Some("{}"));

    let found = unread(dir.path()).await.unwrap();

    assert_eq!(found.shown, vec![message(1, "inv-opus", &"ż".repeat(MAX_TEXT_CHARS))]);
}

#[tokio::test]
async fn unread_messages_older_than_the_part_of_the_log_that_is_read_make_the_list_incomplete() {
    let filler = "x".repeat(4096);
    let count = WINDOW_BYTES / 4096 + 10;
    let lines: Vec<String> = (1..=count)
        .map(|id| line(id, "inv-opus", "claude-1", &filler, ""))
        .chain(std::iter::once(line(count + 1, "inv-opus", "deepseek-1", "newest", "")))
        .collect();
    let dir = bus(&lines, Some("{}"));

    let found = unread(dir.path()).await.unwrap();

    assert_eq!(found.shown, vec![message(count + 1, "inv-opus", "newest")]);
    assert_eq!(found.completeness, Completeness::Newest);
}

#[tokio::test]
async fn a_line_that_is_not_a_message_is_an_error_with_its_number() {
    let dir = bus(
        &[line(1, "inv-opus", "deepseek-1", "fine", ""), "{oops".to_owned()],
        Some("{}"),
    );

    let error = unread(dir.path()).await.unwrap_err();

    assert!(matches!(error, UnreadMessagesError::Line { line: 2, .. }), "{error:?}");
}

#[tokio::test]
async fn a_state_that_is_not_json_is_an_error_naming_the_file() {
    let dir = bus(&[line(1, "inv-opus", "deepseek-1", "fine", "")], None);
    std::fs::write(dir.path().join("state.json"), "{not json").unwrap();

    let error = unread(dir.path()).await.unwrap_err();

    assert!(
        matches!(&error, UnreadMessagesError::State { path, .. } if *path == dir.path().join("state.json")),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_log_that_is_not_utf8_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), b"\xff\xfe\n").unwrap();

    let error = unread(dir.path()).await.unwrap_err();

    assert!(matches!(error, UnreadMessagesError::NotUtf8 { .. }), "{error:?}");
}

#[tokio::test]
async fn a_log_that_cannot_be_opened_is_a_read_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("messages.jsonl")).unwrap();

    let error = unread(dir.path()).await.unwrap_err();

    assert!(matches!(error, UnreadMessagesError::Read { .. }), "{error:?}");
}
