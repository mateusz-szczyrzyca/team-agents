//! Reviewed-head gate provenance and its absence.
use super::*;
use agent_core::config::Language;
use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
use std::path::Path;

#[tokio::test]
async fn reviewers_reuse_only_the_gate_of_the_reviewed_head() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let head: CommitId = "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap();
    let mut record = GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: head.clone(),
            finished_at: "2026-11-19T09:30:00Z".parse().unwrap(),
            outcome: GateOutcome::Passed,
            log: "/tmp/tasks/task-1/gate-2.log".parse().unwrap(),
        },
    };
    assert_eq!(
        review_gate(&texts, &head, Some(&record)).unwrap(),
        include_str!("gate-present.md").trim_end()
    );
    record.result.head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap();
    for record in [Some(&record), None] {
        assert_eq!(
            review_gate(&texts, &head, record).unwrap(),
            include_str!("gate-absent.md").trim_end()
        );
    }
}
