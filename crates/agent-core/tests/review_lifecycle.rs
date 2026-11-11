//! The life of an item's reviews through the checked handle of a loaded state: a review that is pending when the
//! round starts receives its outcome, a rejection sends the item back for a fix pass, and nothing the handle accepts
//! can be saved as a state that does not load again.

#[cfg(test)]
mod review_lifecycle {
    use agent_core::ids::{ItemId, TeamId};
    use agent_core::state::{
        AdvanceError, Completion, ItemPhase, PhaseKind, PullRequest, Review, ReviewOutcome, State, SubmitReviewError,
        Timestamp,
    };
    use agent_core::status::ReviewVerdict;

    /// The state, as shipped with the tests.
    const FIXTURE: &str = include_str!("fixtures/state-v1.json");

    /// The team of the fixture.
    fn team() -> TeamId {
        "team-20261111-193000-a1b2".parse().unwrap()
    }

    /// The item of the fixture that is in review: one approval in, one review pending.
    fn item() -> ItemId {
        "1.2".parse().unwrap()
    }

    /// The reviewer of the fixture whose review is pending.
    fn pending_reviewer() -> agent_core::ids::InvestigatorName {
        "inv-gemini".parse().unwrap()
    }

    /// A moment after the fixture's last review.
    fn later() -> Timestamp {
        "2026-11-11T20:00:00Z".parse().unwrap()
    }

    /// The pull request of the item.
    fn pull_request(state: &State) -> PullRequest {
        match state.team(&team()).unwrap().item(item()).unwrap().phase() {
            ItemPhase::InReview { pr, .. } => pr.clone(),
            other => panic!("the item is not in review: {other:?}"),
        }
    }

    /// The reviews of the round that is in review.
    fn round(state: &State) -> Vec<Review> {
        match state.team(&team()).unwrap().item(item()).unwrap().phase() {
            ItemPhase::InReview { reviews, .. } => reviews.clone(),
            other => panic!("the item is not in review: {other:?}"),
        }
    }

    /// Rule: a pending review receives the reviewer's outcome, and a rejection then sends the item back, with the
    /// round recorded as a fix pass.
    #[test]
    fn a_pending_review_receives_a_rejection_and_the_item_goes_back_for_a_fix_pass() {
        let mut state = State::from_json(FIXTURE).unwrap();
        let pr = pull_request(&state);

        state
            .team_mut(&team())
            .unwrap()
            .item_mut(item())
            .unwrap()
            .submit_review(&pending_reviewer(), ReviewVerdict::RequestChanges, later())
            .unwrap();
        let submitted = round(&state);
        let result = state
            .team_mut(&team())
            .unwrap()
            .item_mut(item())
            .unwrap()
            .advance(ItemPhase::Fixing {
                assignee: "codex-1".parse().unwrap(),
                branch: pr.branch.clone(),
                pr,
            });

        assert_eq!(
            submitted[1].outcome,
            ReviewOutcome::Submitted {
                verdict: ReviewVerdict::RequestChanges,
                at: later()
            }
        );
        assert_eq!(result, Ok(()));
        let item = state.team(&team()).unwrap().item(item()).unwrap();
        assert_eq!(item.fix_passes().len(), 2);
        assert_eq!(item.fix_passes()[1].requested_by, submitted);
        assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    }

    #[test]
    fn a_review_that_is_already_submitted_is_not_overwritten() {
        let mut state = State::from_json(FIXTURE).unwrap();
        let before = state.clone();

        let result = state
            .team_mut(&team())
            .unwrap()
            .item_mut(item())
            .unwrap()
            .submit_review(&"inv-codex".parse().unwrap(), ReviewVerdict::RequestChanges, later());

        assert_eq!(
            result,
            Err(SubmitReviewError::AlreadySubmitted {
                item: item(),
                reviewer: "inv-codex".parse().unwrap()
            })
        );
        assert_eq!(state, before);
    }

    #[test]
    fn a_reviewer_outside_the_round_cannot_submit() {
        let mut state = State::from_json(FIXTURE).unwrap();
        let before = state.clone();

        let result = state
            .team_mut(&team())
            .unwrap()
            .item_mut(item())
            .unwrap()
            .submit_review(&"inv-opus".parse().unwrap(), ReviewVerdict::Approve, later());

        assert_eq!(
            result,
            Err(SubmitReviewError::UnknownReviewer {
                item: item(),
                reviewer: "inv-opus".parse().unwrap()
            })
        );
        assert_eq!(state, before);
    }

    #[test]
    fn an_item_that_is_not_in_review_has_nothing_to_submit() {
        let mut state = State::from_json(FIXTURE).unwrap();

        let result = state
            .team_mut(&team())
            .unwrap()
            .item_mut("2.1".parse().unwrap())
            .unwrap()
            .submit_review(&pending_reviewer(), ReviewVerdict::Approve, later());

        assert_eq!(
            result,
            Err(SubmitReviewError::NotInReview {
                item: "2.1".parse().unwrap(),
                phase: PhaseKind::Pending
            })
        );
    }

    /// Rule: `listed` is how an item that the task list marked done started; an item that was open is done by its
    /// merge, because a state with it listed is refused when it is read.
    #[test]
    fn an_item_that_was_open_cannot_be_completed_as_listed() {
        let mut state = State::from_json(FIXTURE).unwrap();
        let pr = pull_request(&state);
        let approved: Vec<Review> = round(&state)
            .into_iter()
            .map(|review| Review {
                outcome: ReviewOutcome::Submitted {
                    verdict: ReviewVerdict::Approve,
                    at: later(),
                },
                ..review
            })
            .collect();
        let mut handle = state.team_mut(&team()).unwrap();
        let mut progress = handle.item_mut(item()).unwrap();
        progress
            .advance(ItemPhase::Approved {
                pr: pr.clone(),
                reviews: approved,
            })
            .unwrap();

        let listed = progress.advance(ItemPhase::Done {
            completion: Completion::Listed,
        });
        let still = progress.phase().kind();
        let merged = progress.advance(ItemPhase::Done {
            completion: Completion::Merged { pr, at: later() },
        });

        assert_eq!(listed, Err(AdvanceError::ListedCompletion { item: item() }));
        assert_eq!(still, PhaseKind::Approved);
        assert_eq!(merged, Ok(()));
        assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    }
}
