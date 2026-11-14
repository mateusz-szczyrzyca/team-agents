//! The pull request of an item through the checked handle of a loaded state, as a caller outside the crate sees it:
//! the pull request that was reviewed is the one the item goes back to be fixed on and the one it is recorded as
//! merged with — never another.

#[cfg(test)]
mod pull_request_kept {
    use std::num::NonZeroU32;

    use agent_core::ids::{ItemId, TaskId, TeamId};
    use agent_core::state::{
        AdvanceError, Completion, ItemMut, ItemPhase, PhaseKind, PrNumber, PullRequest, State, Timestamp,
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

    /// The pull request of item 1.2 as it is recorded in its current phase.
    fn recorded(item: &ItemMut<'_>) -> PullRequest {
        match item.phase() {
            ItemPhase::InReview { pr, .. } | ItemPhase::Fixing { pr, .. } | ItemPhase::Approved { pr, .. } => {
                pr.clone()
            }
            other => panic!("item 1.2 is in {:?}", other.kind()),
        }
    }

    /// The pull request `pr` with another number.
    fn replaced(pr: &PullRequest) -> PullRequest {
        let mut other = pr.clone();
        other.number = PrNumber::new(NonZeroU32::new(999).unwrap());
        other
    }

    /// The phase that sends item 1.2 back for a fix pass on `pr`.
    fn fixing_on(pr: PullRequest) -> ItemPhase {
        ItemPhase::Fixing {
            assignee: "codex-1".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
            pr,
        }
    }

    #[test]
    fn a_fix_pass_on_another_pull_request_than_the_reviewed_one_is_refused() {
        with_item(|handle| {
            handle
                .submit_review(&pending_task(), ReviewVerdict::RequestChanges, later())
                .unwrap();
            let before = handle.phase().clone();
            let other = replaced(&recorded(handle));

            let refused = handle.advance(fixing_on(other));

            assert_eq!(refused, Err(AdvanceError::ReviewPassNotKept { item: item() }));
            assert_eq!(handle.phase(), &before);
        });
    }

    #[test]
    fn a_fix_pass_on_the_reviewed_pull_request_is_accepted() {
        with_item(|handle| {
            handle
                .submit_review(&pending_task(), ReviewVerdict::RequestChanges, later())
                .unwrap();
            let reviewed = recorded(handle);

            handle.advance(fixing_on(reviewed.clone())).unwrap();

            assert_eq!(handle.phase(), &fixing_on(reviewed));
        });
    }

    /// Approves item 1.2 with the review pass it has.
    fn approve(handle: &mut ItemMut<'_>) {
        handle
            .submit_review(&pending_task(), ReviewVerdict::Approve, later())
            .unwrap();
        let ItemPhase::InReview { pr, reviews } = handle.phase().clone() else {
            panic!("item 1.2 is not in review");
        };
        handle.advance(ItemPhase::Approved { pr, reviews }).unwrap();
    }

    #[test]
    fn an_approved_item_is_not_recorded_as_merged_with_another_pull_request() {
        with_item(|handle| {
            approve(handle);
            let before = handle.phase().clone();
            let other = replaced(&recorded(handle));

            let refused = handle.advance(ItemPhase::Done {
                completion: Completion::Merged { pr: other, at: later() },
            });

            assert_eq!(refused, Err(AdvanceError::ReviewPassNotKept { item: item() }));
            assert_eq!(handle.phase(), &before);
            assert_eq!(handle.phase().kind(), PhaseKind::Approved);
        });
    }

    #[test]
    fn an_approved_item_is_recorded_as_merged_with_its_pull_request() {
        with_item(|handle| {
            approve(handle);
            let approved = recorded(handle);
            let done = ItemPhase::Done {
                completion: Completion::Merged {
                    pr: approved,
                    at: later(),
                },
            };

            handle.advance(done.clone()).unwrap();

            assert_eq!(handle.phase(), &done);
        });
    }
}
