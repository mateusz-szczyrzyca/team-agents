//! The lifecycle of an item through the checked handles of a loaded state, as a caller outside the crate sees it: the
//! outcome of a pending review is recorded, and no advance leaves a state that cannot be loaded again.

#[cfg(test)]
mod item_lifecycle {
    use agent_core::ids::{ItemId, TaskId, TeamId};
    use agent_core::state::{
        AdvanceError, Completion, ItemMut, ItemPhase, PhaseKind, Review, ReviewOutcome, State, SubmitReviewError,
        Timestamp,
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

    /// The task of the reviewer who approved item 1.2.
    fn approving_task() -> TaskId {
        "task-20261111-193007-k3x9".parse().unwrap()
    }

    /// A time after every time of the fixture's item 1.2.
    fn later() -> Timestamp {
        "2026-11-11T20:30:00Z".parse().unwrap()
    }

    /// Runs `check` on item 1.2 of a freshly loaded state, then asserts that the state saved after it loads again.
    fn with_item(check: impl FnOnce(&mut ItemMut<'_>)) {
        let mut state = State::from_json(FIXTURE).unwrap();
        check(&mut state.team_mut(&team()).unwrap().item_mut(item()).unwrap());
        let saved = state.to_json().unwrap();

        assert_eq!(State::from_json(&saved).unwrap(), state);
    }

    /// The reviews of the review pass that item 1.2 is in.
    fn review_pass(item: &ItemMut<'_>) -> Vec<Review> {
        let ItemPhase::InReview { reviews, .. } = item.phase() else {
            panic!("item 1.2 is not in review");
        };
        reviews.clone()
    }

    /// The `Fixing` phase that follows item 1.2's review.
    fn fixing(item: &ItemMut<'_>) -> ItemPhase {
        let ItemPhase::InReview { pr, .. } = item.phase() else {
            panic!("item 1.2 is not in review");
        };
        ItemPhase::Fixing {
            assignee: "codex-1".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
            pr: pr.clone(),
        }
    }

    /// The reviews `before`, with the outcome of the review of `task` replaced by `verdict` given at [`later`].
    fn with_outcome(before: &[Review], task: &TaskId, verdict: ReviewVerdict) -> Vec<Review> {
        before
            .iter()
            .cloned()
            .map(|mut review| {
                if &review.task == task {
                    review.outcome = ReviewOutcome::Submitted { verdict, at: later() };
                }
                review
            })
            .collect()
    }

    #[test]
    fn a_pending_review_receives_its_rejection_and_the_item_goes_back_for_a_fix_pass() {
        with_item(|item| {
            let passes_before = item.fix_passes().len();
            let reviews_before = review_pass(item);

            item.submit_review(&pending_task(), ReviewVerdict::RequestChanges, later())
                .unwrap();
            let next = fixing(item);
            item.advance(next.clone()).unwrap();

            assert_eq!(item.phase(), &next);
            assert_eq!(item.fix_passes().len(), passes_before + 1);
            assert_eq!(
                item.fix_passes().last().unwrap().requested_by,
                with_outcome(&reviews_before, &pending_task(), ReviewVerdict::RequestChanges)
            );
        });
    }

    #[test]
    fn a_submitted_outcome_is_kept_in_the_round_beside_the_other_review() {
        with_item(|item| {
            let reviews_before = review_pass(item);
            assert_eq!(
                reviews_before
                    .iter()
                    .map(|review| review.task.clone())
                    .collect::<Vec<_>>(),
                [approving_task(), pending_task()]
            );

            item.submit_review(&pending_task(), ReviewVerdict::Approve, later())
                .unwrap();

            assert_eq!(
                review_pass(item),
                with_outcome(&reviews_before, &pending_task(), ReviewVerdict::Approve)
            );
        });
    }

    #[test]
    fn two_approvals_let_the_item_move_to_approved() {
        with_item(|item| {
            item.submit_review(&pending_task(), ReviewVerdict::Approve, later())
                .unwrap();
            let ItemPhase::InReview { pr, reviews } = item.phase().clone() else {
                panic!("item 1.2 is not in review");
            };

            item.advance(ItemPhase::Approved { pr, reviews }).unwrap();

            assert_eq!(item.phase().kind(), PhaseKind::Approved);
        });
    }

    #[test]
    fn an_outcome_cannot_be_recorded_twice_for_one_review() {
        with_item(|item| {
            let before = review_pass(item);

            let again = item.submit_review(&approving_task(), ReviewVerdict::RequestChanges, later());

            assert_eq!(
                again,
                Err(SubmitReviewError::AlreadySubmitted {
                    item: item.id(),
                    task: approving_task()
                })
            );
            assert_eq!(review_pass(item), before);
        });
    }

    #[test]
    fn an_outcome_of_a_task_that_was_not_asked_for_a_review_is_refused() {
        with_item(|item| {
            let stranger: TaskId = "task-20261111-193059-k3x9".parse().unwrap();
            let before = review_pass(item);

            let refused = item.submit_review(&stranger, ReviewVerdict::Approve, later());

            assert_eq!(
                refused,
                Err(SubmitReviewError::UnknownReview {
                    item: item.id(),
                    task: stranger
                })
            );
            assert_eq!(review_pass(item), before);
        });
    }

    #[test]
    fn an_item_that_is_not_in_review_receives_no_outcome() {
        let mut state = State::from_json(FIXTURE).unwrap();
        let mut team = state.team_mut(&team()).unwrap();
        let mut pending = team.item_mut("2.1".parse().unwrap()).unwrap();

        let refused = pending.submit_review(&pending_task(), ReviewVerdict::Approve, later());

        assert_eq!(
            refused,
            Err(SubmitReviewError::NotInReview {
                item: pending.id(),
                phase: PhaseKind::Pending
            })
        );
        assert_eq!(pending.phase(), &ItemPhase::Pending);
    }

    /// Moves item 1.2 to approved with both reviews approving.
    fn approve(item: &mut ItemMut<'_>) {
        item.submit_review(&pending_task(), ReviewVerdict::Approve, later())
            .unwrap();
        let ItemPhase::InReview { pr, reviews } = item.phase().clone() else {
            panic!("item 1.2 is not in review");
        };
        item.advance(ItemPhase::Approved { pr, reviews }).unwrap();
    }

    #[test]
    fn an_item_that_was_open_cannot_be_completed_as_listed_and_stays_approved() {
        with_item(|item| {
            approve(item);
            let before = item.phase().clone();

            let refused = item.advance(ItemPhase::Done {
                completion: Completion::Listed,
            });

            assert_eq!(refused, Err(AdvanceError::ListedCompletion { item: item.id() }));
            assert_eq!(item.phase(), &before);
        });
    }

    #[test]
    fn an_approved_item_completes_by_its_merge_and_the_state_loads_again() {
        with_item(|item| {
            approve(item);
            let ItemPhase::Approved { pr, .. } = item.phase().clone() else {
                panic!("item 1.2 is not approved");
            };

            item.advance(ItemPhase::Done {
                completion: Completion::Merged { pr, at: later() },
            })
            .unwrap();

            assert_eq!(item.phase().kind(), PhaseKind::Done);
        });
    }
}
