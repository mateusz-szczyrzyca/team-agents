//! Which review a report answers, when the items of one pull request stand at different fix passes.
use agent_core::state::{PullRequest, Timestamp};

use super::*;

/// A pull request of the fixture's shape.
fn pull_request() -> PullRequest {
    serde_json::from_value(serde_json::json!({
        "number": 82,
        "branch": "agents/codex-1/item-1-3",
        "opened_at": "2026-11-11T19:40:00Z"
    }))
    .unwrap()
}

#[test]
fn a_report_is_recorded_in_the_pass_of_the_most_fixed_item_when_the_joined_item_comes_first() {
    let requested_at: Timestamp = "2026-11-18T19:31:00Z".parse().unwrap();
    let offered = |item: &str, pass| Offered {
        requested_at,
        item: item.parse().unwrap(),
        pr: pull_request(),
        reviewer: "inv-kimi".parse().unwrap(),
        pass,
    };

    let asked = answer_among(
        vec![offered("2.1", 0), offered("1.3", 1)],
        "2026-11-18T19:32:00Z".parse().unwrap(),
    )
    .unwrap();

    assert_eq!(asked.passes, 1);
    assert_eq!(
        asked.items,
        vec!["2.1".parse::<ItemId>().unwrap(), "1.3".parse().unwrap()]
    );
}

use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, investigator, member, pr, review, run, solo_state, task, team_id};
use agent_core::state::{Findings, NewRun, RunEnd, State};
use agent_core::status::{RunOutcome, TaskStatus};
use std::path::Path;
use std::sync::Arc;

/// A reduced copy of the evidence: the same numbered findings, but explanation 28 exists in only the first item.
fn evidence_state() -> State {
    let ledgers: Vec<Findings> = serde_json::from_str(include_str!("evidence-ledgers.json")).unwrap();
    let mut state = solo_state();
    state
        .add_reviewer(&team_id(0), member(3, investigator("inv-1")))
        .unwrap();
    state
        .add_reviewer(&team_id(0), member(4, investigator("inv-2")))
        .unwrap();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    for (index, (id, ledger)) in ["1.1", "1.2"].into_iter().zip(ledgers).enumerate() {
        let id = id.parse().unwrap();
        team.open_round(
            format!("round-20261111-15300{index}-test").parse().unwrap(),
            Some(id),
            at(1),
        )
        .unwrap();
        team.advance_item(
            id,
            ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: pr(1).branch,
            },
        )
        .unwrap();
        team.advance_item(
            id,
            ItemPhase::InReview {
                pr: pr(1),
                reviews: vec![review("inv-1", 3), review("inv-2", 4)],
            },
        )
        .unwrap();
        let mut item = team.item_mut(id).unwrap();
        item.start_findings();
        *item.findings_mut().unwrap() = ledger;
    }
    team.begin_run(
        &task(3),
        NewRun {
            id: run(3),
            at: at(21),
            session: None,
            rounds: vec![],
        },
    )
    .unwrap();
    let (_, mut reviewer) = team.find_mut(&task(3)).unwrap();
    reviewer.transition(TaskStatus::Running).unwrap();
    reviewer
        .end_run(
            at(22),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .unwrap();
    reviewer.transition(TaskStatus::Waiting).unwrap();
    drop(team);
    state
}

/// The evidence's open answers, retaining its refused answer on line 19 and invented evidence text.
fn evidence_report() -> String {
    let state = evidence_state();
    let ledger = state.team(&team_id(0)).unwrap().items()[0].findings().unwrap();
    let mut lines = vec!["VERDICT: REQUEST_CHANGES".to_owned()];
    for entry in ledger
        .entries()
        .iter()
        .filter(|entry| entry.status == agent_core::state::FindingStatus::Open)
    {
        if entry.number.get() != 28 {
            lines.push(format!("FINDING {}: OPEN — regression remains", entry.number));
        }
    }
    while lines.len() < 18 {
        lines.push(String::new());
    }
    lines.push("FINDING 28: ACCEPTED — the helper change is required to compile".to_owned());
    lines.push("STATUS: DONE".to_owned());
    lines.join("\n")
}

async fn pulls(owner: &Owner) -> Pulls {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    Pulls {
        texts: Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        checks: Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        settings: agent_runtime::github::GitHubSettings::default(),
        token: agent_core::config::Secret::try_from("invented-test-token".to_owned()).unwrap(),
        trusted: vec![],
    }
}

#[tokio::test]
async fn evidence_report_is_bounced_for_the_second_item_without_recording_either_item() {
    let state = evidence_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.pulls = Some(pulls(&owner).await);
    let located = owner.locate(&task(3)).unwrap();
    let before = state.team(&team_id(0)).unwrap().items().to_vec();

    let result = owner.review_ended(&task(3), &located, Some(&evidence_report())).await;

    assert!(matches!(result, Ok(())), "{result:?}");
    assert_eq!(owner.parts.state.team(&team_id(0)).unwrap().items(), before);
    assert_eq!(owner.watch.bounces.get(&task(3)), Some(&1));
    assert!(matches!(owner.reasons.get(&task(3)), Some(Why::ReviewFormat(_))));
    assert_eq!(
        owner.parts.state.find_task(&task(3)).unwrap().member.status(),
        TaskStatus::Queued
    );
}

#[tokio::test]
async fn recording_refusal_is_bounced_even_when_the_caller_bypasses_validation() {
    let state = evidence_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let pulls = pulls(&owner).await;
    let located = owner.locate(&task(3)).unwrap();
    let asked = owner.answered_by(&team_id(0), &task(3), at(21)).unwrap();
    let before = state.team(&team_id(0)).unwrap().items().to_vec();

    let result = owner
        .take_verdict(
            &pulls,
            &task(3),
            &located,
            &asked,
            ReviewVerdict::RequestChanges,
            &evidence_report(),
        )
        .await;

    assert!(matches!(result, Ok(())), "{result:?}");
    assert_eq!(owner.parts.state.team(&team_id(0)).unwrap().items(), before);
    assert_eq!(owner.watch.bounces.get(&task(3)), Some(&1));
}
