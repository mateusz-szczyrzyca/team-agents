//! An item that joins a fix pass of a pull request does not hide the pass and the findings of the item being fixed.
use agent_core::review_findings::{Finding, Severity};
use agent_core::state::{FindingStatus, Findings, ItemPhase, ItemProgress, RecordedFinding, State};
use std::num::NonZeroU32;

use super::prompts::verification_of;

/// The fixture's team with item 1.3 fixed once (one fix pass, one open finding) and item 2.1 joined without a pass of
/// its own but with a copy of the pull request's ledger: the two items of one pull request.
fn fixed_and_joined() -> (State, ItemProgress, ItemProgress) {
    let mut state = State::from_json(include_str!(
        "../../../../../../agent-core/tests/fixtures/state-before-findings.json"
    ))
    .unwrap();
    let team = state.teams()[0].id().clone();
    let (fixed, joined) = ("1.3".parse().unwrap(), "2.1".parse().unwrap());
    let ItemPhase::InReview { pr, .. } = state.team(&team).unwrap().item(fixed).unwrap().phase() else {
        panic!("fixture item is in review")
    };
    let pr = pr.clone();
    let entry = RecordedFinding {
        number: NonZeroU32::MIN,
        finding: Finding {
            severity: Severity::Major,
            rule: "ERR-3".into(),
            place: "src/lib.rs:1".into(),
            text: "broken → fix it".into(),
            check: "read src/lib.rs:1".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status: FindingStatus::Open,
    };
    let ledger = Findings::new(vec![entry]).unwrap();
    {
        let mut found = state.team_mut(&team).unwrap();
        let mut item = found.item_mut(fixed).unwrap();
        item.submit_review(
            &"task-20261111-193008-k3x9".parse().unwrap(),
            agent_core::status::ReviewVerdict::RequestChanges,
            "2026-11-18T19:29:00Z".parse().unwrap(),
        )
        .unwrap();
        item.advance(ItemPhase::Fixing {
            assignee: "inv-opus".parse().unwrap(),
            branch: pr.branch.clone(),
            pr: pr.clone(),
        })
        .unwrap();
        item.start_findings();
        *item.findings_mut().unwrap() = ledger.clone();
        let mut item = found.item_mut(joined).unwrap();
        item.advance(ItemPhase::InProgress {
            assignee: "inv-opus".parse().unwrap(),
            branch: pr.branch.clone(),
        })
        .unwrap();
        item.start_findings();
        *item.findings_mut().unwrap() = ledger;
    }
    let team = state.team(&team).unwrap();
    let (fixed, joined) = (team.item(fixed).unwrap().clone(), team.item(joined).unwrap().clone());
    (state, fixed, joined)
}

#[test]
fn the_request_has_the_pass_and_the_findings_of_the_fixed_item_when_the_joined_item_comes_first() {
    let (_state, fixed, joined) = fixed_and_joined();

    let verification = verification_of(&[&joined, &fixed]);

    assert_eq!(verification.pass, fixed.fix_passes().len() + 1);
    assert!(verification.pass > joined.fix_passes().len() + 1);
    assert_eq!(verification.findings, fixed.findings());
    assert!(verification.findings.is_some());
}

#[test]
fn an_item_reopened_for_review_after_a_merge_conflict_pass_gets_a_verification_pass_and_is_not_independent() {
    use agent_core::config::ReviewCount;
    use agent_core::state::{Review, ReviewOutcome};

    let (mut state, _fixed, _joined) = fixed_and_joined();
    let team_id = state.teams()[0].id().clone();
    let item_id = "2.1".parse().unwrap();
    let task: agent_core::ids::TaskId = "task-20261111-193008-k3x9".parse().unwrap();
    {
        let mut team = state.team_mut(&team_id).unwrap();
        let pr = match team.item("1.3".parse().unwrap()).unwrap().phase() {
            ItemPhase::Fixing { pr, .. } => pr.clone(),
            other => panic!("unexpected phase: {other:?}"),
        };
        let mut item = team.item_mut(item_id).unwrap();
        item.advance(ItemPhase::InReview {
            pr,
            reviews: Vec::new(),
        })
        .unwrap();
        item.ask_reviews(vec![Review {
            reviewer: "inv-codex".parse().unwrap(),
            task: task.clone(),
            requested_at: "2026-11-18T19:29:00Z".parse().unwrap(),
            outcome: ReviewOutcome::Pending,
        }])
        .unwrap();
        item.start_findings();
        *item.findings_mut().unwrap() = Findings::independent(ReviewCount::TWO);
        assert_eq!(
            item.begin_merge_conflict(
                "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
                "2026-11-18T19:29:00Z".parse().unwrap(),
            ),
            1
        );
        item.end_merge_conflict();
    }
    let team = state.team(&team_id).unwrap();
    let item = team.item(item_id).unwrap();

    assert_eq!(item.fix_passes().len(), 0);
    assert_eq!(item.merge_passes(), 1);
    assert_eq!(verification_of(&[item]).pass, 2);
    assert!(!crate::daemon::review::independent_first_review(team, &task));
}
