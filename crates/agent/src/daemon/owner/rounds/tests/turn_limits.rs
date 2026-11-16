//! A real daemon with fake CLIs and GitHub: bounded continuation and its recorded explanation.

use agent_core::history::RunWhy;
use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{MergeMode, RunEnd};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_runtime::history::Reader;

use super::rig::{PATIENCE, Rig, investigator, normal};
use crate::client::Connection;

/// A scratch daemon with an injected clock and a configured turn budget.
async fn rig(scenario: &'static str, resumes: u32) -> Rig {
    let mut rig = Rig::new(
        &[
            normal("claude-1", scenario),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    let path = rig.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        path,
        format!("{text}\n[limits]\nmax_turns = 50\nturn_limit_resumes = {resumes}\n"),
    )
    .unwrap();
    rig
}

/// Read exactly the status API that `agent status` uses, rejecting even an intermediate failure.
async fn done_without_failure(rig: &Rig) {
    tokio::time::timeout(PATIENCE, async {
        loop {
            let reply = Connection::connect(&rig.home.socket_file())
                .await
                .unwrap()
                .request(Client::Cli, Command::Status(StatusRequest { target: None }))
                .await
                .unwrap();
            let Reply::Teams { teams } = reply else {
                panic!("history must be healthy: {reply:?}")
            };
            let member = teams[0].members.iter().find(|m| m.agent.name() == "claude-1").unwrap();
            assert_ne!(member.status, TaskStatus::Failed, "agent status reported failure");
            if member.status == TaskStatus::Done {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the resumed run must finish");
}

#[tokio::test]
async fn turn_limit_resumes_same_session_then_done_with_history_and_short_prompt() {
    let mut rig = rig("turn-limit,done", 3).await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
        .await;
    done_without_failure(&rig).await;
    rig.stop().await.unwrap();
    assert_eq!(rig.runs(&tasks[0]), 2);
    let rows = Reader::open(&rig.home.history_file())
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].end.as_ref().unwrap().value.end, RunEnd::TurnLimit);
    assert_eq!(rows[1].start.value.why, RunWhy::TurnLimit);
    assert_eq!(
        rows[1].end.as_ref().unwrap().value.end,
        RunEnd::Reported {
            outcome: RunOutcome::Done
        }
    );
    assert_eq!(rows[0].start.value.session, rows[1].start.value.session);
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-2")).unwrap();
    let session = rows[0].start.value.session.as_ref().unwrap().as_str();
    assert!(args.contains(&format!("--resume\n{session}\n")), "{args}");
    let prompt = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-prompt-2")).unwrap();
    let expected = include_str!("../../../../../../../prompts/harness/run/resume-turn-limit.md");
    assert_eq!(prompt.trim_end(), expected.trim_end());
    assert_eq!(rows[1].start.value.prompt.trim_end(), expected.trim_end());
    let database = std::fs::read(rig.home.history_file()).unwrap();
    assert!(
        !database
            .windows(b"ghp_test_token_0123".len())
            .any(|w| w == b"ghp_test_token_0123")
    );
}

#[tokio::test]
async fn configured_max_turns_reaches_the_cli_process() {
    let mut rig = rig("done", 3).await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
        .await;
    done_without_failure(&rig).await;
    rig.stop().await.unwrap();
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-1")).unwrap();
    assert!(args.contains("--max-turns\n50\n"), "{args}");
}

#[tokio::test]
async fn one_resume_allows_two_turn_limits_then_fails_and_notifies_once() {
    let mut rig = rig("turn-limit", 1).await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
        .await;
    rig.state_where("the resume budget to fail the task", |state| {
        state.find_task(&tasks[0]).unwrap().member.status() == TaskStatus::Failed
    })
    .await;
    rig.stop().await.unwrap();
    assert_eq!(rig.runs(&tasks[0]), 2);
    let notices: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|n| n.contains("turn limit"))
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(
        notices[0].contains("2 consecutive") && notices[0].contains("1 automatic"),
        "{:?}",
        notices[0]
    );
    let rows = Reader::open(&rig.home.history_file())
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r.end.as_ref().unwrap().value.end == RunEnd::TurnLimit)
    );
}

#[tokio::test]
async fn exhausted_team_member_notifies_the_coordinator_on_the_bus() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "turn-limit"),
            investigator("inv-coord", "done"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    let path = rig.home.config_file();
    let config = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, format!("{config}\n[limits]\nturn_limit_resumes = 0\n")).unwrap();
    rig.start().await;
    let (_, tasks) = rig
        .run(
            &["inv-coord", "claude-1"],
            super::rig::ONE,
            MergeMode::Autopilot,
            Some("inv-coord"),
        )
        .await;
    let member_task = tasks
        .iter()
        .find(|id| rig.state().find_task(id).unwrap().member.agent().name() == "claude-1")
        .unwrap()
        .clone();
    rig.status_is(&member_task, TaskStatus::Failed).await;
    rig.stop().await.unwrap();
    let messages = std::fs::read_to_string(rig.home.root().join("bus/messages.jsonl")).unwrap();
    assert!(
        messages.contains("\"to\":\"inv-coord\"") && messages.contains("claude-1"),
        "{messages}"
    );
}
