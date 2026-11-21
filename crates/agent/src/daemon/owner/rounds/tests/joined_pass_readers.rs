//! The readers of a pull request's fix pass for an item that joined the pull request later and has fewer fix passes of
//! its own: item 1.1 of the evidence is in its third fix pass, item 2.1 in its second, both on pull request 132 with
//! the same ledger. Each reader must count the pull request's pass, 3, for both items.
use std::sync::Arc;

use agent_core::decide::fix::Answers;
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::state::{Findings, ItemPhase, State};
use agent_core::status::ReviewVerdict;
use serde_json::{Value, json};

use crate::daemon::owner::Owner;
use crate::daemon::owner::fixture::{Daemon, Sinks};
use crate::daemon::owner::rounds::Pulls;
use crate::daemon::owner::teams::Look;

/// The team of the evidence.
fn team() -> TeamId {
    "team-20261119-131420-gt1d".parse().unwrap()
}

/// The item with the most fix passes, then the one that joined later.
fn items() -> [ItemId; 2] {
    ["1.1".parse().unwrap(), "2.1".parse().unwrap()]
}

/// The two reviewers whose reviews of pull request 132 are pending in the evidence.
fn reviewers() -> [TaskId; 2] {
    [
        "task-20261119-170919-quvr".parse().unwrap(),
        "task-20261119-153246-oe7a".parse().unwrap(),
    ]
}

/// A ledger of the protocol with explanations whose fix pass 3 of the lead is answered, with `entries`,
/// `explanations`, `opinions` and `fixed` as the state keeps them.
fn ledger(entries: &Value, explanations: &Value, opinions: &Value, fixed: &Value) -> Findings {
    serde_json::from_value(json!({
        "review_count": 2,
        "disputes": {
            "explanations": explanations,
            "raised": [],
            "answered_pass": 3,
            "bounces": 0,
            "checked_run": null,
            "correction": null
        },
        "verification": { "reviewed_head": null, "lead_message": null, "extra_fix_passes": 0 },
        "entries": entries,
        "opinions": opinions,
        "fixed": fixed,
        "pending": {},
        "comment": null
    }))
    .unwrap()
}

/// One recorded finding of `severity` and `status`, numbered `number`.
fn entry(number: u32, severity: &str, status: &str) -> Value {
    json!({
        "number": number,
        "finding": {
            "severity": severity,
            "rule": "ERR-3",
            "place": "src/lib.rs:1",
            "text": "the problem — ünïcode → the fix",
            "check": "read src/lib.rs:1"
        },
        "reporter": "inv-gale",
        "pass": 1,
        "status": status
    })
}

/// The second opinion on finding 1 asked in fix pass 3, reported with `outcome`.
fn opinion(outcome: &str) -> Value {
    json!({
        "1": {
            "request": { "pass": 3, "head": "a".repeat(40), "reason": "the defect does not hold" },
            "stage": {
                "stage": "reported",
                "investigator": "inv-fir",
                "task": "task-20261119-180000-op1n",
                "report": { "outcome": outcome, "evidence": "ran the check" }
            }
        }
    })
}

/// The evidence after both reviewers asked for changes: items 1.1 (three fix passes) and 2.1 (two) are fixed on pull
/// request 132, both with `findings`.
fn fixing(findings: &Findings) -> State {
    let mut state = State::from_json(include_str!(
        "../../../../../../agent-core/tests/fixtures/state-split-explanations.json"
    ))
    .unwrap();
    {
        let mut found = state.team_mut(&team()).unwrap();
        let at = "2026-11-19T17:40:00Z".parse().unwrap();
        for id in items() {
            let mut progress = found.item_mut(id).unwrap();
            for task in reviewers() {
                progress
                    .submit_review(&task, ReviewVerdict::RequestChanges, at)
                    .unwrap();
            }
            let ItemPhase::InReview { pr, .. } = progress.phase().clone() else {
                panic!("item {id} of the evidence is in review")
            };
            progress
                .advance(ItemPhase::Fixing {
                    assignee: "lead-ada".parse().unwrap(),
                    branch: pr.branch.clone(),
                    pr,
                })
                .unwrap();
            *progress.findings_mut().unwrap() = findings.clone();
        }
    }
    let passes: Vec<_> = items()
        .iter()
        .map(|id| state.team(&team()).unwrap().item(*id).unwrap().fix_passes().len())
        .collect();
    assert_eq!(passes, vec![3, 2], "the joined item has one fix pass fewer of its own");
    state
}

/// An owner of `state` with the autopilot and its real texts, and the daemon home that keeps it.
async fn owner_of(state: &State) -> (Daemon, Owner, Sinks, Pulls) {
    let daemon = Daemon::new(state, None);
    let secrets = daemon.home.root().join("secrets.toml");
    std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
    std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let (owner, sinks) = daemon.bare_owner().await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    let pulls = Pulls::new(
        Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        &owner.parts.settings,
    )
    .unwrap()
    .unwrap();
    (daemon, owner.with_pulls(pulls.clone()), sinks, pulls)
}

#[tokio::test]
async fn the_update_of_a_pull_request_is_not_held_once_the_response_to_its_pass_is_taken_for_a_joined_item() {
    let findings = ledger(&json!([entry(1, "MAJOR", "open")]), &json!({}), &json!({}), &json!({}));
    let (_daemon, owner, _sinks, _pulls) = owner_of(&fixing(&findings)).await;

    assert!(!owner.held_for_response(&team(), &items()));
    assert!(!owner.held_for_response(&team(), &items()[1..]));
}

#[tokio::test]
async fn an_unchanged_fix_pass_whose_findings_are_all_explained_is_accounted_for_a_joined_item() {
    // The lead fixed finding 1 in fix pass 2 and explained it in fix pass 3; fix pass 2 is the joined item's own count.
    let findings = ledger(
        &json!([entry(1, "MAJOR", "open")]),
        &json!({ "1": { "why": "the units are explicit", "status": "pending", "votes": {} } }),
        &json!({}),
        &json!({ "2": [1] }),
    );
    let (_daemon, owner, _sinks, _pulls) = owner_of(&fixing(&findings)).await;

    assert_eq!(owner.unchanged_answers(&team(), &items()), Answers::Accounted);
    assert_eq!(owner.unchanged_answers(&team(), &items()[1..]), Answers::Accounted);
}

#[tokio::test]
async fn a_confirmed_blocker_of_the_pull_requests_pass_is_returned_for_every_item_with_a_joined_one() {
    let findings = ledger(
        &json!([entry(1, "BLOCKER", "open")]),
        &json!({}),
        &opinion("confirmed"),
        &json!({}),
    );
    let (_daemon, mut owner, _sinks, pulls) = owner_of(&fixing(&findings)).await;

    owner.continue_opinions(&pulls, &team(), &mut Look::default()).unwrap();

    let answered: Vec<u32> = items()
        .iter()
        .map(|id| {
            let team = owner.parts.state.team(&team()).unwrap();
            team.item(*id)
                .unwrap()
                .findings()
                .unwrap()
                .disputes()
                .unwrap()
                .answered_pass
        })
        .collect();
    assert_eq!(answered, vec![0, 0]);
}

#[tokio::test]
async fn a_confirmed_blocker_returns_to_the_assigned_member_instead_of_the_coordinator() {
    let findings = ledger(
        &json!([entry(1, "BLOCKER", "open")]),
        &json!({}),
        &opinion("confirmed"),
        &json!({}),
    );
    let mut state = fixing(&findings);
    for id in items() {
        state
            .team_mut(&team())
            .unwrap()
            .item_mut(id)
            .unwrap()
            .reassign("member-bob".parse().unwrap(), "agents/member-bob/fix".parse().unwrap())
            .unwrap();
    }
    let (_daemon, mut owner, _sinks, pulls) = owner_of(&state).await;
    let coordinator = owner.lead_of(&team()).unwrap().task;
    let member: TaskId = "task-20261119-131420-nni1".parse().unwrap();

    owner.continue_opinions(&pulls, &team(), &mut Look::default()).unwrap();

    assert_eq!(
        (
            owner.reasons.contains_key(&member),
            owner.reasons.contains_key(&coordinator)
        ),
        (true, false),
    );
}
