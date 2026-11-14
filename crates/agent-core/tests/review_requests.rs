//! Asking for the reviews of a pull request that is open, through the checked handle of an item, as a caller outside
//! the crate sees it: the review pass of an item in review starts empty and is filled once, by reviews that are all still
//! pending, and the state saved after it loads again.

#[cfg(test)]
mod review_requests {
    use std::num::NonZeroU32;

    use agent_core::ids::{ItemId, TaskId, TeamId};
    use agent_core::state::{
        AskReviewsError, Crew, InvalidReviews, ItemMut, ItemPhase, Member, MergeMode, Participant, PhaseKind, PrNumber,
        PullRequest, Review, ReviewOutcome, State, TaskListText, Team, Timestamp,
    };
    use agent_core::status::ReviewVerdict;

    /// A task list with one item.
    const LIST: &str = "Build the thing.\n\n1. First\n[ ] 1.1 One\n    Do one.\n";

    fn item() -> ItemId {
        "1.1".parse().unwrap()
    }

    fn team_id() -> TeamId {
        "team-20261111-193000-a1b2".parse().unwrap()
    }

    /// The task of the solo agent.
    fn lead_task() -> TaskId {
        "task-20261111-193001-k3x9".parse().unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    /// A state with a solo team whose only item is pending.
    fn state() -> State {
        let lead = Member::new(lead_task(), Participant::Normal("claude-1".parse().unwrap()));
        let team = Team::new(
            team_id(),
            "team-agents".parse().unwrap(),
            at("2026-11-11T19:30:00Z"),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            Crew::solo(lead).unwrap(),
        );
        let mut state = State::new();
        state.insert_team(team).unwrap();
        state
    }

    fn pr() -> PullRequest {
        PullRequest {
            number: PrNumber::new(NonZeroU32::new(7).unwrap()),
            branch: "agents/claude-1/task-1".parse().unwrap(),
            opened_at: at("2026-11-11T19:40:00Z"),
        }
    }

    /// A review by `name`, asked at 19:41, whose outcome is `outcome`.
    fn review(name: &str, number: u32, outcome: ReviewOutcome) -> Review {
        Review {
            reviewer: name.parse().unwrap(),
            task: format!("task-20261111-1931{number:02}-k3x9").parse().unwrap(),
            requested_at: at("2026-11-11T19:41:00Z"),
            outcome,
        }
    }

    fn pending(name: &str, number: u32) -> Review {
        review(name, number, ReviewOutcome::Pending)
    }

    /// Runs `check` on the item of a state whose item is in review with no review asked yet, then asserts that the state
    /// saved after it loads again.
    fn with_open_pull_request(check: impl FnOnce(&mut ItemMut<'_>)) {
        let mut state = state();
        state
            .open_round(
                &team_id(),
                "round-20261111-193030-r4n9".parse().unwrap(),
                Some(item()),
                at("2026-11-11T19:30:30Z"),
            )
            .unwrap();
        {
            let mut team = state.team_mut(&team_id()).unwrap();
            let mut item = team.item_mut(item()).unwrap();
            item.advance(ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task-1-1.1".parse().unwrap(),
            })
            .unwrap();
            item.advance(ItemPhase::InReview {
                pr: pr(),
                reviews: Vec::new(),
            })
            .unwrap();
            check(&mut item);
        }
        let saved = state.to_json().unwrap();

        assert_eq!(State::from_json(&saved).unwrap(), state);
    }

    fn review_pass(item: &ItemMut<'_>) -> Vec<Review> {
        let ItemPhase::InReview { reviews, .. } = item.phase() else {
            panic!("the item is not in review");
        };
        reviews.clone()
    }

    #[test]
    fn the_reviews_asked_of_a_pull_request_fill_its_empty_round() {
        with_open_pull_request(|item| {
            let asked = vec![pending("inv-codex", 10), pending("inv-gemini", 11)];

            item.ask_reviews(asked.clone()).unwrap();

            assert_eq!(review_pass(item), asked);
            assert_eq!(item.phase().kind(), PhaseKind::InReview);
        });
    }

    #[test]
    fn a_review_asked_stays_pending_until_its_outcome_is_submitted() {
        with_open_pull_request(|item| {
            let asked = vec![pending("inv-codex", 10), pending("inv-gemini", 11)];
            item.ask_reviews(asked.clone()).unwrap();

            item.submit_review(&asked[0].task, ReviewVerdict::Approve, at("2026-11-11T20:00:00Z"))
                .unwrap();

            let reviews = review_pass(item);
            assert_eq!(reviews[1], asked[1]);
            assert!(matches!(reviews[0].outcome, ReviewOutcome::Submitted { .. }));
        });
    }

    #[test]
    fn a_round_that_has_its_reviews_is_not_asked_again() {
        with_open_pull_request(|item| {
            item.ask_reviews(vec![pending("inv-codex", 10), pending("inv-gemini", 11)])
                .unwrap();

            let refused = item.ask_reviews(vec![pending("inv-opus", 12)]).unwrap_err();

            assert_eq!(refused, AskReviewsError::AlreadyAsked { item: item.id() });
            assert_eq!(review_pass(item).len(), 2);
        });
    }

    #[test]
    fn no_reviews_are_not_a_request() {
        with_open_pull_request(|item| {
            let refused = item.ask_reviews(Vec::new()).unwrap_err();

            assert_eq!(refused, AskReviewsError::NoReviews { item: item.id() });
            assert_eq!(review_pass(item), []);
        });
    }

    #[test]
    fn a_review_that_has_its_outcome_already_is_not_asked() {
        with_open_pull_request(|item| {
            let done = review(
                "inv-gemini",
                11,
                ReviewOutcome::Submitted {
                    verdict: ReviewVerdict::Approve,
                    at: at("2026-11-11T19:50:00Z"),
                },
            );

            let refused = item.ask_reviews(vec![pending("inv-codex", 10), done]).unwrap_err();

            assert_eq!(
                refused,
                AskReviewsError::NotPending {
                    item: item.id(),
                    reviewer: "inv-gemini".parse().unwrap(),
                }
            );
            assert_eq!(review_pass(item), []);
        });
    }

    #[test]
    fn one_reviewer_is_not_asked_twice() {
        with_open_pull_request(|item| {
            let refused = item
                .ask_reviews(vec![pending("inv-codex", 10), pending("inv-codex", 11)])
                .unwrap_err();

            assert_eq!(
                refused,
                AskReviewsError::Reviews {
                    item: item.id(),
                    source: InvalidReviews::DuplicateReviewer {
                        reviewer: "inv-codex".parse().unwrap(),
                    },
                }
            );
            assert_eq!(review_pass(item), []);
        });
    }

    #[test]
    fn an_item_that_is_not_in_review_receives_no_review_request() {
        let mut state = state();
        let mut team = state.team_mut(&team_id()).unwrap();
        let mut item = team.item_mut(item()).unwrap();

        let refused = item.ask_reviews(vec![pending("inv-codex", 10)]).unwrap_err();

        assert_eq!(
            refused,
            AskReviewsError::NotInReview {
                item: item.id(),
                phase: PhaseKind::Pending,
            }
        );
    }
}
