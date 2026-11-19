//! Turn exhaustion participates in a whole round and remains independent of history availability.

use agent_core::history::{HistoryState, RunWhy};
use agent_core::kinds::Role;
use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{ItemPhase, MergeMode, RunEnd};
use agent_core::status::TaskStatus;
use agent_core::story::{self, PromptView, Selection, StepView, StoryOptions};
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

#[tokio::test]
async fn a_turn_limited_round_continues_through_both_reviews_and_merge() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "turn-limit,work"),
            investigator("inv-r1", "turn-limit,done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |p| matches!(p, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let round = state.teams()[0].rounds()[0].id();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let rows = reader.runs_of_round(round).unwrap();
    assert_eq!(rows.len(), 5);
    for entry in std::fs::read_dir(rig.home.root()).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_str().unwrap().starts_with("history.db") {
            let bytes = std::fs::read(path).unwrap();
            assert!(
                !bytes
                    .windows(b"ghp_test_token_0123".len())
                    .any(|w| w == b"ghp_test_token_0123")
            );
        }
    }
    assert_eq!(reader.runs_of_team(&team).unwrap(), rows);
    let records = reader.round_records(round).unwrap();
    let selection = Selection::Round(round.clone());
    let story = story::render(
        &records,
        &selection,
        StoryOptions {
            prompts: PromptView::Preview,
            steps: StepView::LeftOut,
        },
    );
    let endings: Vec<_> = story
        .lines()
        .filter(|line| line.trim() == "ended: turn limit reached")
        .map(str::trim)
        .collect();
    let starts: Vec<_> = story
        .lines()
        .filter(|line| line.contains("because the previous run reached its turn limit"))
        .map(|line| line.split_once(" as ").unwrap().1)
        .collect();
    let normalized = format!("{}\n{}\n", endings.join("\n"), starts.join("\n"));
    assert_eq!(
        normalized,
        include_str!("../../../../../tests/fixtures/turn-limit/story.txt")
    );
    let json: Vec<serde_json::Value> = story::json_lines(&records, &selection)
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        json.iter()
            .filter(|r| r["record"]["end"] == serde_json::json!({"kind": "turn_limit"}))
            .count(),
        2
    );
    assert_eq!(
        json.iter()
            .filter(|r| r["record"]["why"] == serde_json::json!({"kind": "turn_limit"}))
            .count(),
        2
    );
    for role in [Role::Solo, Role::Reviewer] {
        let first = rows
            .iter()
            .find(|r| r.start.value.role == role && r.end.as_ref().unwrap().value.end == RunEnd::TurnLimit)
            .unwrap();
        let resumed = rows
            .iter()
            .find(|r| r.start.value.task == first.start.value.task && r.start.value.why == RunWhy::TurnLimit)
            .unwrap();
        assert_eq!(first.start.value.session, resumed.start.value.session);
        assert_eq!(first.start.value.rounds, resumed.start.value.rounds);
    }
}

#[tokio::test]
async fn turn_limit_continuation_survives_an_unwritable_or_newer_history_and_reports_loss() {
    for newer in [false, true] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", "turn-limit,done"),
                investigator("inv-r1", "done"),
                investigator("inv-r2", "done"),
            ],
            5,
            false,
        )
        .await;
        rig.pin_turn_limit_clock();
        let path = rig.home.history_file();
        let before = if newer {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.pragma_update(None, "user_version", 11).unwrap();
            drop(raw);
            Some(std::fs::read(&path).unwrap())
        } else {
            std::fs::create_dir(&path).unwrap();
            None
        };
        rig.start().await;
        let (_, tasks) = rig
            .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
            .await;
        rig.status_is(&tasks[0], TaskStatus::Done).await;
        let reply = Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
        rig.stop().await.unwrap();
        assert_eq!(rig.runs(&tasks[0]), 2);
        let Reply::Overview { history, .. } = reply else {
            panic!("missing history health: {reply:?}")
        };
        assert!(matches!(history.state, HistoryState::Unavailable { .. }));
        assert!(history.lost >= 4, "both starts and ends must be reported lost");
        if let Some(before) = before {
            assert_eq!(std::fs::read(path).unwrap(), before);
        }
    }
}

#[tokio::test]
async fn zero_resumes_notifies_with_the_exact_exhaustion_text_and_restart_does_not_repeat_it() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "turn-limit"),
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
        .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Failed).await;
    rig.stop().await.unwrap();
    let notices: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|n| n.contains("turn limit"))
        .collect();
    assert_eq!(notices.len(), 1);
    let (expected_title, expected_body) = include_str!("../../../../../tests/fixtures/turn-limit/exhausted.txt")
        .trim_end()
        .split_once('\n')
        .unwrap();
    assert!(notices[0].starts_with("❌ example\n"));
    assert!(notices[0].contains("\nRepo: example\n"));
    assert!(notices[0].contains("\nAgent: claude-1 (claude)\n"));
    assert!(notices[0].contains(&format!("\nTask: {}\n", tasks[0])));
    assert!(notices[0].contains("\nStatus: ❌ failed\n"));
    assert!(notices[0].contains(&format!("\nNote: {expected_title}\n\n")));
    let body_part = notices[0].split("\n\n").nth(1).unwrap();
    assert_eq!(body_part.replace(tasks[0].as_str(), "<TASK>"), expected_body);
    assert_eq!(rig.runs(&tasks[0]), 1);
    rig.start().await;
    rig.status_is(&tasks[0], TaskStatus::Failed).await;
    rig.stop().await.unwrap();
    assert_eq!(
        rig.notifications()
            .into_iter()
            .filter(|n| n.contains("turn limit"))
            .collect::<Vec<_>>(),
        notices
    );
    assert_eq!(rig.runs(&tasks[0]), 1);
}

#[tokio::test]
async fn resumed_prompt_matches_its_golden_and_a_resumed_failure_still_fails() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "turn-limit,done:STATUS: FAILED"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    rig.start().await;
    let (_, tasks) = rig
        .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Failed).await;
    rig.stop().await.unwrap();
    assert_eq!(rig.runs(&tasks[0]), 2);
    let prompt = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-prompt-2")).unwrap();
    assert_eq!(
        prompt.trim_end(),
        include_str!("../../../../../tests/fixtures/turn-limit/resume.txt").trim_end()
    );
}

#[tokio::test]
async fn quota_in_a_turn_limited_run_pauses_instead_of_spending_a_resume() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "turn-limit-quota"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], "Finish the work.", MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Paused).await;
    rig.stop().await.unwrap();
    assert_eq!(rig.runs(&tasks[0]), 1);
    let rows = Reader::open(&rig.home.history_file())
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].end.as_ref().unwrap().value.end,
        RunEnd::QuotaExhausted { resets_at: None }
    );
}
