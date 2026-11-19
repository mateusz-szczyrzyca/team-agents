//! A schema-10 state written by main 1c73e339 preserves an open second fix pass and opinion.
use agent_core::state::{ActivityEnd, ActivityOutcome, ActivityRequest, State};
use std::num::NonZeroU32;

#[test]
fn previous_schema_rebuilds_crew_first_review_verification_and_second_opinion() {
    let old = include_str!("fixtures/state-before-activities.json");
    let state = State::from_json(old).unwrap();
    let team = &state.teams()[0];
    assert_eq!(team.activities().len(), 4);
    let entries = team.activities();
    assert_eq!(entries[0].investigator().as_str(), "inv-opus");
    assert_eq!(entries[0].task().as_str(), "task-20261111-193001-k3x9");
    assert_eq!(entries[0].request(), &ActivityRequest::TeamMember);
    assert_eq!(entries[0].asked_at(), "2026-11-11T19:30:00Z".parse().unwrap());
    assert_eq!(entries[0].outcome(), &ActivityOutcome::Working);
    for (entry, request, asked, ended) in [
        (
            &entries[1],
            ActivityRequest::FirstReview,
            "2026-11-11T19:41:00Z",
            "2026-11-11T19:44:00Z",
        ),
        (
            &entries[2],
            ActivityRequest::Verification { pass: NonZeroU32::MIN },
            "2026-11-11T19:47:00Z",
            "2026-11-11T19:48:00Z",
        ),
    ] {
        assert_eq!(entry.investigator().as_str(), "inv-codex");
        assert_eq!(entry.task().as_str(), "task-20261111-193007-k3x9");
        assert_eq!(entry.request(), &request);
        assert_eq!(entry.asked_at(), asked.parse().unwrap());
        assert_eq!(
            entry.outcome(),
            &ActivityOutcome::Finished {
                at: ended.parse().unwrap(),
                end: ActivityEnd::Reported
            }
        );
    }
    assert_eq!(
        entries[3].request(),
        &ActivityRequest::SecondOpinion {
            pass: NonZeroU32::new(2).unwrap(),
            finding: NonZeroU32::MIN
        }
    );
    assert_eq!(entries[3].investigator().as_str(), "inv-gemini");
    assert_eq!(entries[3].task().as_str(), "task-20261111-193008-k3x9");
    assert_eq!(entries[3].asked_at(), "2026-11-11T19:49:00Z".parse().unwrap());
    assert_eq!(entries[3].outcome(), &ActivityOutcome::Working);
    for entry in entries {
        assert_eq!(entry.rounds(), &["round-20261111-193000-a1b4".parse().unwrap()]);
    }
    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(written["schema_version"], 11);
    written["schema_version"] = serde_json::json!(10);
    written["teams"][0].as_object_mut().unwrap().remove("activities");
    assert_eq!(written, serde_json::from_str::<serde_json::Value>(old).unwrap());
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    assert_eq!(State::from_json(old).unwrap(), state);
}

#[test]
fn schema_ten_permissions_survive_activity_reconstruction() {
    let text = include_str!("fixtures/state-before-activities.json").replace(
        "Coordinator: inv-opus\\n\\nBuild the thing.",
        "Coordinator: inv-opus\\nAllow: docs/*.txt\\n\\nBuild the thing.",
    );
    let state = State::from_json(&text).unwrap();
    let permissions: Vec<_> = state.teams()[0]
        .allowances()
        .iter()
        .map(agent_core::task_allowance::TaskAllowance::as_str)
        .collect();
    assert_eq!(permissions, ["docs/*.txt"]);
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn two_reports_of_one_reviewer_for_one_item_stay_two_activities_when_their_request_times_are_equal() {
    let state = State::from_json(include_str!("fixtures/state-before-disputes.json")).unwrap();
    let of_codex: Vec<_> = state.teams()[0]
        .activities()
        .iter()
        .filter(|activity| activity.investigator().as_str() == "inv-codex")
        .collect();
    assert_eq!(of_codex.len(), 2);
    assert_eq!(of_codex[0].request(), &ActivityRequest::FirstReview);
    assert_eq!(
        of_codex[0].outcome(),
        &ActivityOutcome::Finished {
            at: "2026-11-11T19:44:00Z".parse().unwrap(),
            end: ActivityEnd::Reported
        }
    );
    assert_eq!(
        of_codex[1].request(),
        &ActivityRequest::Verification { pass: NonZeroU32::MIN }
    );
    assert_eq!(
        of_codex[1].outcome(),
        &ActivityOutcome::Finished {
            at: "2026-11-11T19:50:00Z".parse().unwrap(),
            end: ActivityEnd::Reported
        }
    );
}
