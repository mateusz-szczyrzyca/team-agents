//! A member that the lead's message started again may report `DONE` before the lead's run is over: the newer notice does
//! not erase what the lead said about the work before it.

use super::*;

const TEAM: &str = "team-20261120-191109-jeuf";
const TASK: &str = "task-20261120-191109-qesn";

/// A directory with `lines` as the log of the bus.
fn bus(lines: &[String]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    dir
}

/// A direct message of the lead to `claude-1`.
fn chat(id: u64, text: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"lead","to":"claude-1","kind":"chat","text":{text}}}"#)
}

/// The notice that tells the lead that `claude-1` finished item 1.1.
fn notice(id: u64) -> String {
    format!(
        r#"{{"id":{id},"ts":"2026-11-20 14:00:00","from":"agentbus","to":"lead","kind":"script","text":"done","data":{{"notice":"member_finished","fields":{{"item":"1.1","member":"claude-1","branch":"b"}},"body":"done"}}}}"#
    )
}

/// The ids of what the lead said about the work of `claude-1`, by the newest notice the lead answered.
async fn answered(dir: &Path) -> Option<Vec<u64>> {
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
    .unwrap()
    .map(|said| said.iter().map(|message| message.id.get()).collect())
}

#[test]
fn a_newer_notice_that_the_lead_has_not_answered_leaves_the_earlier_answer() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let table = [
        (
            "the member finished again before the lead's run was over",
            vec![notice(1), chat(2, "fix the reader"), notice(3)],
            Some(vec![2]),
        ),
        (
            "two answers, the newest notice answered",
            vec![notice(1), chat(2, "first"), notice(3), chat(4, "second")],
            Some(vec![4]),
        ),
        (
            "two answers, the newest notice not answered yet",
            vec![notice(1), chat(2, "first"), notice(3), chat(4, "second"), notice(5)],
            Some(vec![4]),
        ),
        (
            "several messages of one answer",
            vec![notice(1), chat(2, "a"), chat(3, "b"), notice(4)],
            Some(vec![2, 3]),
        ),
        ("a notice nobody answered", vec![notice(1), notice(2)], Some(vec![])),
    ];
    for (what, lines, expected) in table {
        let dir = bus(&lines);
        assert_eq!(runtime.block_on(answered(dir.path())), expected, "{what}");
    }
}
