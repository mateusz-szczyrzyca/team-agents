//! Migration preserves an open fix pass before bounded opinion assignments existed.
use agent_core::state::{SCHEMA_VERSION, State};
use std::num::NonZeroU32;

#[test]
fn schema_eight_active_fix_pass_migrates_without_inventing_opinion_attempts() {
    let old = include_str!("fixtures/state-before-opinion-retries.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 11);
    let ledger = state.teams()[0].items()[1].findings().unwrap();
    assert_eq!(ledger.opinion_attempts(NonZeroU32::MIN), []);
    assert_eq!(ledger.opinions().len(), 0);
    assert_eq!(
        ledger.fixed_answers(NonZeroU32::MIN),
        std::collections::BTreeSet::default()
    );
    assert_eq!(
        state.to_json().unwrap(),
        include_str!("fixtures/state-before-opinion-retries-activities.json")
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    let mut state = state;
    let team_id = state.teams()[0].id().clone();
    let mut team = state.team_mut(&team_id).unwrap();
    let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
    let ledger = item.findings_mut().unwrap();
    ledger
        .answer_opinions(
            "FINDING 1: SECOND OPINION — café boundary",
            NonZeroU32::MIN,
            &"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            agent_core::review_response::opinion::Permission::Enabled,
        )
        .unwrap();
    ledger
        .assign_opinion_at(
            NonZeroU32::MIN,
            "inv-independent".parse().unwrap(),
            "task-20261111-193007-k3x9".parse().unwrap(),
            "2026-11-11T19:50:00Z".parse().unwrap(),
        )
        .unwrap();
    assert_eq!(ledger.opinion_attempts(NonZeroU32::MIN).len(), 1);
    assert_eq!(ledger.disputes().unwrap().answered_pass, 1);
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn round_details_keep_assignment_times_for_status_and_omit_them_for_legacy_findings() {
    let mut state = State::from_json(include_str!("fixtures/state-before-opinion-retries.json")).unwrap();
    let team_id = state.teams()[0].id().clone();
    let mut team = state.team_mut(&team_id).unwrap();
    let item_id = "1.2".parse().unwrap();
    let mut item = team.item_mut(item_id).unwrap();
    let ledger = item.findings_mut().unwrap();
    ledger
        .request_opinion(
            NonZeroU32::MIN,
            agent_core::state::OpinionRequest {
                pass: NonZeroU32::MIN,
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                reason: "café boundary".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    ledger
        .assign_opinion_at(
            NonZeroU32::MIN,
            "inv-independent".parse().unwrap(),
            "task-20261111-193007-k3x9".parse().unwrap(),
            "2026-11-11T19:50:00Z".parse().unwrap(),
        )
        .unwrap();
    let expected = ledger.opinion_attempts(NonZeroU32::MIN).to_vec();
    let team = state.team(&team_id).unwrap();
    let round = team
        .rounds()
        .iter()
        .find(|round| round.item() == Some(item_id))
        .unwrap();
    let detail = agent_core::protocol::RoundDetail::new(team, round, Vec::new());
    assert_eq!(detail.opinion_attempts.get(&NonZeroU32::MIN), Some(&expected));
}

#[test]
fn legacy_assignment_rejects_replacing_a_retained_attempt_without_corrupting_state() {
    use agent_core::state::{OpinionFailure, OpinionRequest, OpinionStateError};
    let mut state = State::from_json(include_str!("fixtures/state-before-opinion-retries.json")).unwrap();
    let team_id = state.teams()[0].id().clone();
    let number = NonZeroU32::MIN;
    let first: agent_core::ids::TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    let mut team = state.team_mut(&team_id).unwrap();
    let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
    let ledger = item.findings_mut().unwrap();
    ledger
        .request_opinion(
            number,
            OpinionRequest {
                pass: number,
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                reason: "café boundary".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    ledger
        .assign_opinion_at(
            number,
            "inv-first".parse().unwrap(),
            first.clone(),
            "2026-11-11T19:50:00Z".parse().unwrap(),
        )
        .unwrap();
    ledger.fail_opinion(number, &first, OpinionFailure::Failed).unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.assign_opinion(
            number,
            "inv-next".parse().unwrap(),
            "task-20261111-193008-k3x9".parse().unwrap()
        ),
        Err(OpinionStateError { number })
    );
    assert_eq!(*ledger, before);
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
