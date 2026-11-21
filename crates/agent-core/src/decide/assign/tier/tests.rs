//! Offered agent tiers and coordinator fallback, through the public choice API.

use super::*;

#[test]
fn chooses_only_members_at_or_above_each_requested_tier() {
    let candidates = [
        Participant::Worker("worker".parse().expect("name")),
        Participant::Normal("normal".parse().expect("name")),
        Participant::Investigator("investigator".parse().expect("name")),
    ];
    for (required, eligible) in [
        (Some(Tier::Worker), [true, true, true]),
        (Some(Tier::Normal), [false, true, true]),
        (Some(Tier::Investigator), [false, false, true]),
        (None, [false, true, true]),
    ] {
        for (candidate, fits) in candidates.iter().zip(eligible) {
            let offered = std::slice::from_ref(candidate);
            let expected = if fits {
                TierChoice::Member(candidate)
            } else {
                TierChoice::NoAgent {
                    required: required.unwrap_or(Tier::Normal),
                }
            };
            assert_eq!(choose_agent(required, offered, None), expected);
        }
    }
}

#[test]
fn coordinator_is_used_only_when_no_member_fits() {
    let members = [Participant::Normal("member".parse().expect("name"))];
    for coordinator in [
        Participant::Worker("coordinator-worker".parse().expect("name")),
        Participant::Normal("coordinator-normal".parse().expect("name")),
        Participant::Investigator("coordinator-investigator".parse().expect("name")),
    ] {
        assert_eq!(
            choose_agent(Some(Tier::Normal), &members, Some(&coordinator)),
            TierChoice::Member(&members[0])
        );
        let expected = match coordinator.tier() {
            Tier::Investigator => TierChoice::Coordinator(&coordinator),
            Tier::Normal | Tier::Worker => TierChoice::NoAgent {
                required: Tier::Investigator,
            },
        };
        assert_eq!(
            choose_agent(Some(Tier::Investigator), &members, Some(&coordinator)),
            expected
        );
    }
}

#[test]
fn coordinator_alone_is_checked_against_every_requirement() {
    let coordinator = Participant::Normal("coordinator".parse().expect("name"));
    for (required, expected) in [
        (Some(Tier::Worker), TierChoice::Coordinator(&coordinator)),
        (Some(Tier::Normal), TierChoice::Coordinator(&coordinator)),
        (None, TierChoice::Coordinator(&coordinator)),
        (
            Some(Tier::Investigator),
            TierChoice::NoAgent {
                required: Tier::Investigator,
            },
        ),
    ] {
        assert_eq!(choose_agent(required, &[], Some(&coordinator)), expected);
    }
}

#[test]
fn no_candidates_returns_the_effective_requirement() {
    for required in [None, Some(Tier::Worker), Some(Tier::Normal), Some(Tier::Investigator)] {
        assert_eq!(
            choose_agent(required, &[], None),
            TierChoice::NoAgent {
                required: required.unwrap_or(Tier::Normal)
            }
        );
    }
}

#[test]
fn first_fitting_member_wins_in_offered_order() {
    let members = [
        Participant::Worker("worker".parse().expect("name")),
        Participant::Investigator("first".parse().expect("name")),
        Participant::Normal("second".parse().expect("name")),
    ];
    assert_eq!(choose_agent(None, &members, None), TierChoice::Member(&members[1]));
}
