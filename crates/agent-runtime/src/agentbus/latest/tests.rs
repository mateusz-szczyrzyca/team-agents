use super::*;

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn team() -> TeamId {
    "team-20261119-131420-8hbi".parse().unwrap()
}

/// A line of the log: `extra` is more JSON fields, with a leading comma.
fn line(id: u64, from: &str, to: &str, extra: &str) -> String {
    format!(r#"{{"id":{id},"ts":"2026-11-19 14:00:00","from":"{from}","to":"{to}","text":"zażółć"{extra}}}"#)
}

/// Writes the lines as the log of a bus directory and reads the newest message of `claude-1` from it.
async fn latest(lines: &[String]) -> Result<Option<MessageNote>, LatestMessageError> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    latest_message_for(dir.path(), &agent("claude-1"), &team()).await
}

fn note(id: u64, from: &str) -> Option<MessageNote> {
    Some(MessageNote {
        id: NonZeroU64::new(id).unwrap(),
        from: from.to_owned(),
    })
}

#[tokio::test]
async fn a_bus_without_a_log_has_no_message_for_anybody() {
    let dir = tempfile::tempdir().unwrap();

    let found = latest_message_for(dir.path(), &agent("claude-1"), &team())
        .await
        .unwrap();

    assert_eq!(found, None);
}

#[tokio::test]
async fn the_newest_message_sent_to_the_agent_is_the_one_found() {
    let lines = [
        line(1, "human", "claude-1", ""),
        line(2, "claude-2", "claude-1", ""),
        line(3, "claude-2", "claude-3", ""),
    ];

    assert_eq!(latest(&lines).await.unwrap(), note(2, "claude-2"));
}

#[tokio::test]
async fn messages_to_the_team_and_to_everybody_count_and_other_teams_do_not() {
    let to_team = format!("team:{}", team());
    let to_other = "team:team-20261119-000000-zzzz";

    assert_eq!(
        latest(&[line(1, "human", "claude-1", ""), line(2, "human", &to_team, "")])
            .await
            .unwrap(),
        note(2, "human")
    );
    assert_eq!(
        latest(&[line(1, "human", "claude-1", ""), line(2, "agentbus", "all", "")])
            .await
            .unwrap(),
        note(2, "agentbus")
    );
    assert_eq!(
        latest(&[line(1, "human", "claude-1", ""), line(2, "human", to_other, "")])
            .await
            .unwrap(),
        note(1, "human")
    );
}

#[tokio::test]
async fn what_the_agent_sent_and_what_was_sent_quietly_wakes_nobody() {
    let lines = [
        line(1, "human", "claude-1", ""),
        line(2, "claude-1", "team:team-20261119-131420-8hbi", ""),
        line(3, "claude-2", "claude-1", r#","quiet":true"#),
    ];

    assert_eq!(latest(&lines).await.unwrap(), note(1, "human"));
}

#[tokio::test]
async fn a_log_with_nothing_for_the_agent_has_no_message_for_it() {
    assert_eq!(latest(&[line(1, "human", "claude-2", "")]).await.unwrap(), None);
}

#[tokio::test]
async fn a_line_that_is_no_message_is_a_failure_with_its_number() {
    let lines = [line(1, "human", "claude-1", ""), "not json".to_owned()];

    let error = latest(&lines).await.unwrap_err();

    assert!(matches!(error, LatestMessageError::Line { line: 2, .. }), "{error:?}");
}

#[tokio::test]
async fn a_log_longer_than_the_window_is_read_from_its_end_without_its_cut_line() {
    let dir = tempfile::tempdir().unwrap();
    let filler = line(1, "human", "claude-2", &format!(r#","pad":"{}""#, "x".repeat(1024)));
    let mut text = String::new();
    while u64::try_from(text.len()).unwrap() < super::WINDOW_BYTES + 4096 {
        text.push_str(&filler);
        text.push('\n');
    }
    text.push_str(&line(9, "human", "claude-1", ""));
    text.push('\n');
    std::fs::write(dir.path().join("messages.jsonl"), text).unwrap();

    let found = latest_message_for(dir.path(), &agent("claude-1"), &team())
        .await
        .unwrap();

    assert_eq!(found, note(9, "human"));
}

#[tokio::test]
async fn a_log_that_is_not_utf8_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), [0xff, b'\n']).unwrap();

    let error = latest_message_for(dir.path(), &agent("claude-1"), &team())
        .await
        .unwrap_err();

    assert!(matches!(error, LatestMessageError::NotUtf8 { .. }), "{error:?}");
}
