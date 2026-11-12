//! The approval of an item through the checked handle of a loaded state, as a caller outside the crate sees it: the
//! item is approved with the round it has — its pull request and its reviews as recorded — and with nothing else.

#[cfg(test)]
mod approval_round {
    use std::num::NonZeroU32;

    use agent_core::ids::{ItemId, TaskId, TeamId};
    use agent_core::state::{
        AdvanceError, ItemMut, ItemPhase, PhaseKind, PrNumber, Review, ReviewOutcome, State, Timestamp,
    };
    use agent_core::status::ReviewVerdict;

    /// The state, as shipped with the tests: item 1.2 is in review with one approval and one review still pending.
    const FIXTURE: &str = include_str!("fixtures/state-v1.json");

    /// The team of the fixture whose item 1.2 is in review.
    fn team() -> TeamId {
        "team-20261111-193000-a1b2".parse().unwrap()
    }

    /// The item of the fixture's team that is in review.
    fn item() -> ItemId {
        "1.2".parse().unwrap()
    }

    /// The task of the reviewer whose review of item 1.2 is pending.
    fn pending_task() -> TaskId {
        "task-20261111-193008-k3x9".parse().unwrap()
    }

    /// A time after every time of the fixture's item 1.2.
    fn later() -> Timestamp {
        "2026-11-11T20:30:00Z".parse().unwrap()
    }

    /// Runs `check` on item 1.2 of a freshly loaded state.
    fn with_item(check: impl FnOnce(&mut ItemMut<'_>)) {
        let mut state = State::from_json(FIXTURE).unwrap();
        check(&mut state.team_mut(&team()).unwrap().item_mut(item()).unwrap());
    }

    /// The pull request and the reviews of the round item 1.2 is in.
    fn round(item: &ItemMut<'_>) -> (agent_core::state::PullRequest, Vec<Review>) {
        let ItemPhase::InReview { pr, reviews } = item.phase().clone() else {
            panic!("item 1.2 is not in review");
        };
        (pr, reviews)
    }

    /// Tries to approve item 1.2, after both reviews approved, with the round that `tamper` makes of the real one;
    /// asserts that the item is refused with [`AdvanceError::RoundNotKept`] and is left as it was.
    fn assert_refused_with(tamper: impl FnOnce(&mut agent_core::state::PullRequest, &mut Vec<Review>)) {
        with_item(|handle| {
            handle
                .submit_review(&pending_task(), ReviewVerdict::Approve, later())
                .unwrap();
            let (mut pr, mut reviews) = round(handle);
            let before = handle.phase().clone();
            tamper(&mut pr, &mut reviews);

            let refused = handle.advance(ItemPhase::Approved { pr, reviews });

            assert_eq!(refused, Err(AdvanceError::RoundNotKept { item: item() }));
            assert_eq!(handle.phase(), &before);
            assert_eq!(handle.phase().kind(), PhaseKind::InReview);
        });
    }

    #[test]
    fn an_approval_that_drops_a_review_of_the_round_is_refused() {
        assert_refused_with(|_, reviews| {
            reviews.pop();
        });
    }

    #[test]
    fn an_approval_that_invents_a_reviewer_is_refused() {
        assert_refused_with(|_, reviews| {
            let mut invented = reviews[0].clone();
            invented.reviewer = "inv-invented".parse().unwrap();
            invented.task = "task-20261111-194009-zzzz".parse().unwrap();
            reviews.push(invented);
        });
    }

    #[test]
    fn an_approval_with_another_pull_request_is_refused() {
        assert_refused_with(|pr, _| pr.number = PrNumber::new(NonZeroU32::new(999).unwrap()));
    }

    #[test]
    fn an_approval_that_claims_the_outcome_of_a_pending_review_is_refused() {
        with_item(|handle| {
            let (pr, mut reviews) = round(handle);
            let before = handle.phase().clone();
            for review in &mut reviews {
                review.outcome = ReviewOutcome::Submitted {
                    verdict: ReviewVerdict::Approve,
                    at: later(),
                };
            }

            let refused = handle.advance(ItemPhase::Approved { pr, reviews });

            assert_eq!(refused, Err(AdvanceError::RoundNotKept { item: item() }));
            assert_eq!(handle.phase(), &before);
        });
    }

    #[test]
    fn an_approval_with_the_round_as_recorded_is_accepted() {
        with_item(|handle| {
            handle
                .submit_review(&pending_task(), ReviewVerdict::Approve, later())
                .unwrap();
            let (pr, reviews) = round(handle);

            handle
                .advance(ItemPhase::Approved {
                    pr: pr.clone(),
                    reviews: reviews.clone(),
                })
                .unwrap();

            assert_eq!(handle.phase(), &ItemPhase::Approved { pr, reviews });
        });
    }
}
