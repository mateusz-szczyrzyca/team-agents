//! Where the work of a round is, told from what `show` of the round gives and what the history says of a draft, built
//! from the state through the public API.

#[cfg(test)]
#[path = "support/world.rs"]
mod world;

#[cfg(test)]
mod round_work {
    use agent_core::kinds::Role;
    use agent_core::protocol::{RoundDetail, RoundTask};
    use agent_core::round_work::{DraftPull, RoundWork, round_work};
    use agent_core::state::{Completion, ItemPhase, NewRun, ReviewOutcome, RoundEnd, State};
    use agent_core::status::ReviewVerdict;

    use super::world::{at, item, pr, review, round, run, state, task, team_id};

    /// Every answer the history can give about a draft.
    const DRAFTS: [DraftPull; 3] = [DraftPull::Absent, DraftPull::Open, DraftPull::Unknown];

    /// A state whose team 1 has the round 1 open for item 1.2, which is not started.
    fn opened() -> State {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        state
    }

    /// [`opened`] with item 1.2 given to the member.
    fn in_progress() -> State {
        let mut state = opened();
        state
            .team_mut(&team_id(1))
            .unwrap()
            .item_mut(item("1.2"))
            .unwrap()
            .advance(ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();
        state
    }

    /// [`in_progress`] with the pull request 7 open and both reviews asked for, none of them in.
    fn in_review() -> State {
        let mut state = in_progress();
        {
            let mut team = state.team_mut(&team_id(1)).unwrap();
            let mut handle = team.item_mut(item("1.2")).unwrap();
            handle
                .advance(ItemPhase::InReview {
                    pr: pr(7),
                    reviews: Vec::new(),
                })
                .unwrap();
            handle
                .ask_reviews(vec![
                    review("inv-codex", 7, ReviewOutcome::Pending),
                    review("inv-gemini", 8, ReviewOutcome::Pending),
                ])
                .unwrap();
        }
        state
    }

    /// [`in_review`] after `inv-codex` asked for changes: the team fixes.
    fn fixing() -> State {
        let mut state = in_review();
        {
            let mut team = state.team_mut(&team_id(1)).unwrap();
            let mut handle = team.item_mut(item("1.2")).unwrap();
            handle
                .submit_review(&task(7), ReviewVerdict::RequestChanges, at(30))
                .unwrap();
            handle
                .advance(ItemPhase::Fixing {
                    assignee: "codex-1".parse().unwrap(),
                    branch: pr(7).branch,
                    pr: pr(7),
                })
                .unwrap();
        }
        state
    }

    /// [`in_review`] with both reviews approving and the item approved.
    fn approved() -> State {
        let mut state = in_review();
        {
            let mut team = state.team_mut(&team_id(1)).unwrap();
            let mut handle = team.item_mut(item("1.2")).unwrap();
            for reviewer in [7, 8] {
                handle
                    .submit_review(&task(reviewer), ReviewVerdict::Approve, at(30))
                    .unwrap();
            }
            let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
                panic!("in review");
            };
            handle.advance(ItemPhase::Approved { pr: pr(7), reviews }).unwrap();
        }
        state
    }

    /// The detail of the round 1 of team 1 with the tasks `tasks`.
    fn detail_with(state: &State, tasks: Vec<RoundTask>) -> RoundDetail {
        let team = state.team(&team_id(1)).unwrap();
        RoundDetail::new(team, team.round(&round(1)).unwrap(), tasks)
    }

    /// The detail of the round 1 of team 1 with no tasks.
    fn detail(state: &State) -> RoundDetail {
        detail_with(state, Vec::new())
    }

    /// The reviewer's task 7 of team 1 as a round shows it, with the runs the state has for it.
    fn reviewer_task(state: &State) -> RoundTask {
        let team = state.team(&team_id(1)).unwrap();
        let reviewer = team
            .reviewers()
            .iter()
            .find(|member| member.task() == &task(7))
            .unwrap();
        RoundTask {
            task: task(7),
            role: Role::Reviewer,
            agent: reviewer.agent().clone(),
            worktree: "/home/dev/agents/worktrees/team-agents/task-7".parse().unwrap(),
            branch: "agents/inv-codex/task-7".parse().unwrap(),
            runs: reviewer.runs().to_vec(),
        }
    }

    // Rule: before the reviews are asked for the state knows no pull request, and the history says whether a draft is open.
    #[test]
    fn work_without_a_pull_request_in_the_state_is_local_or_a_draft_as_the_history_says() {
        for state in [opened(), in_progress()] {
            let shown = detail(&state);

            assert_eq!(round_work(&shown, DraftPull::Absent), Some(RoundWork::OnlyLocal));
            assert_eq!(round_work(&shown, DraftPull::Open), Some(RoundWork::PullDraft));
            assert_eq!(round_work(&shown, DraftPull::Unknown), Some(RoundWork::LocalOrDraft));
        }
    }

    // Rule: a round for a whole task list has no item and never a pull request of the harness.
    #[test]
    fn the_round_of_a_whole_task_list_is_local() {
        let mut state = state();
        state.open_round(&team_id(3), round(2), None, at(1)).unwrap();
        let team = state.team(&team_id(3)).unwrap();
        let shown = RoundDetail::new(team, team.round(&round(2)).unwrap(), Vec::new());

        assert_eq!(round_work(&shown, DraftPull::Absent), Some(RoundWork::OnlyLocal));
    }

    // Rule: a pull request whose reviews are asked for is ready until a reviewer starts; what the history says of a
    // draft no longer counts.
    #[test]
    fn a_pull_request_no_reviewer_has_started_on_is_ready() {
        let mut state = in_review();
        // The member ran for the round: only a reviewer's run starts the review.
        state
            .begin_run(
                &task(2),
                NewRun {
                    id: run(1),
                    at: at(5),
                    session: None,
                    rounds: vec![round(1)],
                },
            )
            .unwrap();
        let (role, worked) = state.team(&team_id(1)).unwrap().find(&task(2)).unwrap();
        let member = RoundTask {
            task: task(2),
            role,
            agent: worked.agent().clone(),
            worktree: "/home/dev/agents/worktrees/team-agents/task-2".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
            runs: worked.runs().to_vec(),
        };
        let shown = detail_with(&state, vec![member, reviewer_task(&state)]);

        for draft in DRAFTS {
            assert_eq!(round_work(&shown, draft), Some(RoundWork::PullReady), "{draft:?}");
        }
    }

    #[test]
    fn a_pull_request_a_reviewer_runs_for_is_in_review() {
        let mut state = in_review();
        state
            .begin_run(
                &task(7),
                NewRun {
                    id: run(2),
                    at: at(25),
                    session: None,
                    rounds: vec![round(1)],
                },
            )
            .unwrap();
        let shown = detail_with(&state, vec![reviewer_task(&state)]);

        assert_eq!(round_work(&shown, DraftPull::Absent), Some(RoundWork::PullInReview));
    }

    #[test]
    fn a_pull_request_with_a_verdict_in_is_in_review() {
        let mut state = in_review();
        state
            .team_mut(&team_id(1))
            .unwrap()
            .item_mut(item("1.2"))
            .unwrap()
            .submit_review(&task(8), ReviewVerdict::Approve, at(30))
            .unwrap();

        assert_eq!(
            round_work(&detail(&state), DraftPull::Absent),
            Some(RoundWork::PullInReview)
        );
    }

    // Rule: the fixes after a review, and the reviews asked for after them, are all the review of the pull request.
    #[test]
    fn a_pull_request_that_is_being_fixed_or_reviewed_again_is_in_review() {
        let fixing = fixing();
        let mut again = fixing.clone();
        {
            let mut team = again.team_mut(&team_id(1)).unwrap();
            let mut handle = team.item_mut(item("1.2")).unwrap();
            handle
                .advance(ItemPhase::InReview {
                    pr: pr(7),
                    reviews: Vec::new(),
                })
                .unwrap();
            handle
                .ask_reviews(vec![
                    review("inv-codex", 7, ReviewOutcome::Pending),
                    review("inv-gemini", 8, ReviewOutcome::Pending),
                ])
                .unwrap();
        }

        for state in [fixing, again] {
            for draft in DRAFTS {
                assert_eq!(
                    round_work(&detail(&state), draft),
                    Some(RoundWork::PullInReview),
                    "{draft:?}"
                );
            }
        }
    }

    #[test]
    fn an_approved_pull_request_waits_for_its_merge() {
        assert_eq!(
            round_work(&detail(&approved()), DraftPull::Absent),
            Some(RoundWork::PullApproved)
        );
    }

    // Rule: a round nobody works on any more has no place of work: its item is done, or it is closed.
    #[test]
    fn a_round_whose_item_is_done_or_that_is_closed_has_no_work() {
        let mut done = approved();
        done.team_mut(&team_id(1))
            .unwrap()
            .item_mut(item("1.2"))
            .unwrap()
            .advance(ItemPhase::Done {
                completion: Completion::Merged { pr: pr(7), at: at(40) },
            })
            .unwrap();
        let mut closed = in_progress();
        closed
            .team_mut(&team_id(1))
            .unwrap()
            .close_round(&round(1), at(50), RoundEnd::Abandoned)
            .unwrap();

        for state in [done, closed] {
            for draft in DRAFTS {
                assert_eq!(round_work(&detail(&state), draft), None, "{draft:?}");
            }
        }
    }
}
