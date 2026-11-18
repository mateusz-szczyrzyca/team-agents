//! A configured pass waits for every verdict before deciding.

use agent_core::config::ReviewCount;
use agent_core::decide::round::{RoundAction, RoundEvent, RoundPolicy, RoundState, Stage};
use agent_core::state::{MergeMode, PrNumber};
use agent_core::status::ReviewVerdict;
use std::num::NonZeroU32;

#[test]
fn three_reviewers_decide_only_after_the_third_verdict() {
    let pr = PrNumber::new(NonZeroU32::MIN);
    let policy = RoundPolicy {
        merge: MergeMode::Autopilot,
        fix_passes: 5,
    };
    let required = ReviewCount::try_from(3).unwrap();
    for first in [ReviewVerdict::Approve, ReviewVerdict::RequestChanges] {
        let mut state = RoundState::InReview {
            pr,
            passes: 0,
            approvals: vec![],
            changes: vec![],
        };
        for (index, name) in ["a", "b", "c"].into_iter().enumerate() {
            let event = RoundEvent::ReviewSubmitted {
                reviewer: name.parse().unwrap(),
                verdict: if index == 0 { first } else { ReviewVerdict::Approve },
            };
            let step = state.transition_with_reviews(&event, policy, required).unwrap();
            if index < 2 {
                assert_eq!(step.action, RoundAction::AwaitReviews);
            } else {
                match first {
                    ReviewVerdict::Approve => assert_eq!(step.state, RoundState::Approved { pr }),
                    ReviewVerdict::RequestChanges => assert_eq!(
                        step.state,
                        RoundState::MemberWork {
                            stage: Stage::Fixing {
                                pr,
                                pass: NonZeroU32::MIN
                            },
                        }
                    ),
                }
            }
            state = step.state;
        }
    }
}
