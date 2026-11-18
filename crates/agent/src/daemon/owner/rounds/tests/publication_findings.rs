//! Items joining an existing pull request share its findings and original review protocol.
use agent_core::review_findings::{Finding, Severity};
use agent_core::state::{FindingStatus, Findings, ItemPhase, RecordedFinding, State};
use std::num::NonZeroU32;

use super::publish::Published;
use crate::daemon::owner::fixture::Daemon;
use crate::daemon::owner::teams::Look;
use crate::daemon::pr::{Joining, Publication, Publish};

#[tokio::test]
async fn an_item_joining_a_fix_pass_inherits_the_pr_findings_or_legacy_protocol() {
    for modern in [true, false] {
        let mut state = State::from_json(include_str!(
            "../../../../../../agent-core/tests/fixtures/state-before-findings.json"
        ))
        .unwrap();
        let team = state.teams()[0].id().clone();
        let fixed = "1.3".parse().unwrap();
        let joined = "2.1".parse().unwrap();
        let ItemPhase::InReview { pr, .. } = state.team(&team).unwrap().item(fixed).unwrap().phase() else {
            panic!("fixture item is in review")
        };
        let pr = pr.clone();
        let branch = pr.branch.clone();
        let mut ledger = Findings::default();
        ledger.comment = Some(42);
        ledger.entries.push(RecordedFinding {
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
        });
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
                branch: branch.clone(),
                pr: pr.clone(),
            })
            .unwrap();
            if modern {
                item.start_findings();
                *item.findings_mut().unwrap() = ledger.clone();
            }
            found
                .item_mut(joined)
                .unwrap()
                .advance(ItemPhase::InProgress {
                    assignee: "inv-opus".parse().unwrap(),
                    branch: branch.clone(),
                })
                .unwrap();
            found
                .open_round(
                    "round-20261118-193000-b2c3".parse().unwrap(),
                    Some(joined),
                    "2026-11-18T19:30:00Z".parse().unwrap(),
                )
                .unwrap();
        }
        let daemon = Daemon::new(&state, None);
        let (mut owner, _sinks) = daemon.bare_owner().await;
        let publication = Publication {
            head_branch: branch.clone(),
            publish: Publish::Update {
                pr: pr.clone(),
                fixing: vec![fixed],
                joining: vec![Joining { item: joined, branch }],
            },
        };
        let head: agent_core::state::CommitId = "a".repeat(40).parse().unwrap();
        let published = Published {
            pr,
            gated: head.clone(),
            carried: head,
            items: vec![fixed, joined],
        };
        owner
            .record_publication(&team, &publication, &published, &mut Look::default())
            .unwrap();
        let expected = modern.then_some(&ledger);
        assert_eq!(
            owner.parts.state.team(&team).unwrap().item(joined).unwrap().findings(),
            expected
        );
        assert_eq!(
            owner.parts.state.team(&team).unwrap().item(fixed).unwrap().findings(),
            expected
        );
        assert_eq!(
            State::from_json(&owner.parts.state.to_json().unwrap()).unwrap(),
            owner.parts.state
        );
    }
}
