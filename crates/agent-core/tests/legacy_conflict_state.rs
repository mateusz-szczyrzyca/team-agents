//! State written before conflict accounting loads cleanly and preserves spent passes and conflict state.

use agent_core::decide::merge_refusal::{
    ConflictBudget, ConflictPublication, ExhaustedRoundRecovery, GatingStatus, SplitPullStatus,
    recover_exhausted_split_round,
};
use agent_core::state::{ItemPhase, MergeConflict, State};

#[test]
fn state_before_conflict_accounting_loads_with_exact_passes_and_conflict() {
    let text = include_str!("fixtures/state-before-conflict-accounting.json");
    let state = State::from_json(text).expect("state before conflict accounting should load cleanly");
    let team = &state.teams()[0];
    let item = team.item("1.1".parse().unwrap()).unwrap();

    assert!(matches!(item.phase(), ItemPhase::Approved { .. }));
    assert_eq!(item.merge_passes(), 2);
    assert!(matches!(item.merge_conflict(), Some(MergeConflict::AskingHuman { .. })));

    let round = &team.rounds()[0];
    assert_eq!(round.part().map(|k| k.to_string()), Some("a".to_string()));

    // Authenticate real pre-change facts derived from state:
    let is_split_part = team
        .rounds()
        .iter()
        .any(|r| r.part().is_some() && team.items().iter().any(|item| r.item() == Some(item.id())));
    assert!(is_split_part, "round must be recognized as an open split part");

    let has_open_questions = team
        .crew()
        .roster()
        .iter()
        .any(|(_, member)| member.questions().iter().any(agent_core::state::Question::is_open));
    assert!(!has_open_questions, "fixture must have no open human questions");

    // Case 1: When a usable unpublished gated correction exists, recovery resumes merge:
    let unpublished_recovery = recover_exhausted_split_round(SplitPullStatus::Open {
        gating: GatingStatus::Gated,
        budget: ConflictBudget {
            spent: item.merge_passes(),
            limit: 2,
        },
        conflict: ConflictPublication::Unpublished,
    });
    assert_eq!(unpublished_recovery, ExhaustedRoundRecovery::ResumeMerge);

    // Case 2: When no usable correction exists (ungated), round stays held for human:
    let ungated_recovery = recover_exhausted_split_round(SplitPullStatus::Open {
        gating: GatingStatus::Ungated,
        budget: ConflictBudget {
            spent: item.merge_passes(),
            limit: 2,
        },
        conflict: ConflictPublication::Unpublished,
    });
    assert_eq!(ungated_recovery, ExhaustedRoundRecovery::WaitHuman);

    // Case 3: When result was already published and passes are legitimately exhausted, stays held:
    let legitimate_exhaustion = recover_exhausted_split_round(SplitPullStatus::Open {
        gating: GatingStatus::Gated,
        budget: ConflictBudget {
            spent: item.merge_passes(),
            limit: 2,
        },
        conflict: ConflictPublication::Published,
    });
    assert_eq!(legitimate_exhaustion, ExhaustedRoundRecovery::WaitHuman);
}
