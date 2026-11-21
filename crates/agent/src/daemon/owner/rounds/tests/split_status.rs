//! The status of a split whose parts merge one after the other, as the CLI prints it, and the daemon's log.

use super::rig::{ONE, PATIENCE, Rig, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::fixture::{answers, n};
use agent_core::config::Secrets;
use agent_core::ids::TeamId;
use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{AnswerRequest, Client, Command, Reply, StatusRequest, TeamSummary};
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;
use agent_runtime::history::{HistoryLayer, Reader};

/// How a split of `count` parts depends: each part comes after the one before it.
fn approval(count: usize) -> String {
    let keys = ["a", "b"];
    let parts = (0..count)
        .map(|index| {
            let depends: Vec<&str> = index
                .checked_sub(1)
                .and_then(|before| keys.get(before).copied())
                .into_iter()
                .collect();
            serde_json::json!({
                "title":format!("Document {index}"), "delivers":format!("Independent document {index}"),
                "checks":format!("Read document {index}"), "depends_on":depends
            })
        })
        .collect::<Vec<_>>();
    let plan = serde_json::json!({"parent":"1.1", "parts":parts});
    format!("split\n```json\n{plan}\n```")
}

/// Writes the script `part` of the stand-in CLI: every run commits a file named after it, so that each of the member's
/// runs delivers work of its own; the stand-in records the gate across that commit itself.
fn part_script(rig: &Rig, part: &str) {
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    let body = format!(
        "echo \"work of run $(< .fake-runs) for {part}\" > \"work-$(< .fake-runs).txt\"\n\
         git add \"work-$(< .fake-runs).txt\"\n\
         git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m \"Work for {part}\"\n"
    );
    std::fs::write(dir.join(format!("{part}.sh")), body).unwrap();
}

/// The status of the team `team`, as the daemon answers `agent status` and `agent status --running` with.
async fn summary(rig: &Rig, team: &TeamId) -> TeamSummary {
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("the status answers with teams: {reply:?}");
    };
    teams
        .iter()
        .find(|listed| &listed.id == team)
        .cloned()
        .unwrap_or_else(|| panic!("the team {team} is listed: {teams:?}"))
}

/// The lines that the full view of `team` prints of the parts of item 1.1.
fn full_parts(team: &TeamSummary) -> Vec<String> {
    crate::commands::status::render(std::slice::from_ref(team))
        .lines()
        .filter(|line| line.starts_with("    1.1"))
        .map(std::string::ToString::to_string)
        .collect()
}

/// The lines that the running view of `team` prints of the parts of item 1.1.
fn running_parts(team: &TeamSummary) -> Vec<String> {
    crate::commands::status::render_running(std::slice::from_ref(team))
        .lines()
        .filter(|line| line.starts_with("    1.1"))
        .map(std::string::ToString::to_string)
        .collect()
}

/// The daemon's `daemon_task_failed` records of a round that waits for nothing, with its time and round.
fn round_wait_failures(rig: &Rig) -> Vec<String> {
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    logs.into_iter()
        .filter(|entry| entry.event() == LogEvent::DaemonTaskFailed)
        .filter(|entry| entry.fields().get("operation").and_then(|value| value.as_str()) == Some("round_wait"))
        .map(|entry| format!("at {} team {:?}", entry.at(), entry.team().clone()))
        .collect()
}

/// The pull request that the round of item 1.1 of `state` published for the part `part`, if that round closed with it.
fn merged_pull_request(state: &State, team: &TeamId, part: &str) -> Option<u32> {
    state.team(team)?.rounds().iter().find_map(|round| {
        round
            .part()
            .filter(|key| key.to_string() == part)
            .and_then(|_| round.part_pull_request().map(|pr| pr.number.get().get()))
    })
}

/// Waits until the round of the part `part` of item 1.1 merged its pull request, and returns its number.
async fn merged(rig: &Rig, team: &TeamId, part: &'static str) -> u32 {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let state = rig.state();
        if let Some(number) = merged_pull_request(&state, team, part) {
            return number;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the pull request of part {part}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// The lines of the three parts of the sample split in the state `part state`.
fn lines(first: &str, second: &str, third: &str) -> Vec<String> {
    vec![
        format!("    1.1a  {first}  Document 0"),
        format!("    1.1b  {second}  Document 1"),
        format!("    1.1c  {third}  Document 2"),
    ]
}

// Three parts merge one after the other, the daemon restarts after the first: every view prints the same state for a
// part, a closed item keeps no part line in the running view, and no round opened after a merge is reported as
// waiting for nothing.
#[tokio::test]
async fn a_three_part_split_shows_the_same_state_in_every_view_across_merges_and_a_restart() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "script:part-a,script:part-b,script:part-c"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    part_script(&rig, "part-a");
    part_script(&rig, "part-b");
    part_script(&rig, "part-c");
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: n(1),
                answers: answers(&[(1, &approval(3))]),
            }),
        )
        .await
        .unwrap();

    let first = merged(&rig, &team, "a").await;
    let after_first = summary(&rig, &team).await;
    let expected = lines("done", "in progress", "waiting for 1.1b");
    assert_eq!(full_parts(&after_first), expected, "after the first merge");
    assert_eq!(running_parts(&after_first), expected, "after the first merge, running");

    rig.stop().await.unwrap();
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
    let after_restart = summary(&rig, &team).await;
    assert_eq!(full_parts(&after_restart), expected, "after the restart");
    assert_eq!(running_parts(&after_restart), expected, "after the restart, running");

    let second = merged(&rig, &team, "b").await;
    assert_ne!(first, second);
    let after_second = summary(&rig, &team).await;
    let expected = lines("done", "done", "in progress");
    assert_eq!(full_parts(&after_second), expected, "after the second merge");
    assert_eq!(
        running_parts(&after_second),
        expected,
        "after the second merge, running"
    );

    let third = merged(&rig, &team, "c").await;
    let closed = rig
        .item_where("1.1", "all three parts merged", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    assert_eq!(merged_pull_request(&closed, &team, "c"), Some(third));
    let after_all = summary(&rig, &team).await;
    assert_eq!(
        full_parts(&after_all),
        lines("done", "done", "done"),
        "the closed item, full"
    );
    assert!(running_parts(&after_all).is_empty(), "the closed item, running");

    let failures = round_wait_failures(&rig);
    assert!(failures.is_empty(), "{failures:#?}");
    rig.status_is(&tasks[0], TaskStatus::Done).await;
    rig.stop().await.unwrap();
}
