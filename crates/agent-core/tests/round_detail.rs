//! What `show` of a round gives and what a team's summary says of its rounds, built from the state through the public API:
//! the pull request and the review passes of the round's item, the tasks with a run that names the round, and that the
//! events about rounds change no summary on their own.

#[cfg(test)]
#[path = "support/world.rs"]
mod world;

#[cfg(test)]
mod round_detail {
    use agent_core::kinds::Role;
    use agent_core::protocol::{EventKind, RoundDetail, RoundTask, TeamSummary};
    use agent_core::state::{Completion, ItemPhase, NewRun, PhaseKind, Review, ReviewOutcome, RoundEnd, State};
    use agent_core::status::ReviewVerdict;

    use super::world::{at, item, pr, review, round, run, state, task, team_id};

    /// A state whose team 1 has item 1.2 in review under the round 1, with one fix pass behind it and a run of the member and
    /// one of the coordinator naming the round.
    fn in_review() -> State {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        {
            let mut found = state.team_mut(&team_id(1)).unwrap();
            let mut handle = found.item_mut(item("1.2")).unwrap();
            handle
                .advance(ItemPhase::InProgress {
                    assignee: "codex-1".parse().unwrap(),
                    branch: "agents/codex-1/item-1-2".parse().unwrap(),
                })
                .unwrap();
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
            handle
                .submit_review(&task(7), ReviewVerdict::RequestChanges, at(30))
                .unwrap();
            let ItemPhase::InReview { pr: opened, .. } = handle.phase().clone() else {
                panic!("in review");
            };
            handle
                .advance(ItemPhase::Fixing {
                    assignee: "codex-1".parse().unwrap(),
                    branch: opened.branch.clone(),
                    pr: opened.clone(),
                })
                .unwrap();
            handle
                .advance(ItemPhase::InReview {
                    pr: opened,
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
        for (owner, n) in [(2, 1), (1, 2)] {
            state
                .begin_run(
                    &task(owner),
                    NewRun {
                        id: run(n),
                        at: at(5),
                        session: None,
                        rounds: vec![round(1)],
                    },
                )
                .unwrap();
        }
        state
    }

    /// The detail of the round 1 of team 1 with no tasks.
    fn detail(state: &State) -> RoundDetail {
        let team = state.team(&team_id(1)).unwrap();
        RoundDetail::new(team, team.round(&round(1)).unwrap(), Vec::new())
    }

    #[test]
    fn a_round_shows_its_item_its_pull_request_and_its_review_passes_oldest_first() {
        let state = in_review();

        let shown = detail(&state);

        assert_eq!(shown.team, team_id(1));
        let listed = shown.item.as_ref().unwrap();
        assert_eq!((listed.id, listed.title.as_str()), (item("1.2"), "The next item"));
        assert_eq!(listed.status.to_string(), "in_review");
        assert_eq!(shown.pr, Some(pr(7)));
        let verdicts: Vec<Vec<Option<ReviewVerdict>>> = shown
            .review_passes
            .iter()
            .map(|pass| {
                pass.reviews
                    .iter()
                    .map(|found: &Review| match found.outcome {
                        ReviewOutcome::Submitted { verdict, .. } => Some(verdict),
                        ReviewOutcome::Pending => None,
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            verdicts,
            [vec![Some(ReviewVerdict::RequestChanges), None], vec![None, None]]
        );
    }

    #[test]
    fn a_round_of_an_item_that_has_no_pull_request_yet_shows_none_and_a_whole_list_has_no_item() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        state.open_round(&team_id(3), round(2), None, at(1)).unwrap();

        let started = detail(&state);
        let team = state.team(&team_id(3)).unwrap();
        let whole = RoundDetail::new(team, team.round(&round(2)).unwrap(), Vec::new());

        assert_eq!((started.pr, started.review_passes), (None, Vec::new()));
        assert!(started.item.is_some());
        assert_eq!((whole.item, whole.pr, whole.review_passes), (None, None, Vec::new()));
    }

    #[test]
    fn a_merged_item_keeps_its_pull_request_and_its_fix_passes_and_not_the_reviews_of_the_pass_that_was_approved() {
        let mut state = in_review();
        {
            let mut found = state.team_mut(&team_id(1)).unwrap();
            let mut handle = found.item_mut(item("1.2")).unwrap();
            for owner in [7, 8] {
                handle
                    .submit_review(&task(owner), ReviewVerdict::Approve, at(40))
                    .unwrap();
            }
            let ItemPhase::InReview { pr: opened, reviews } = handle.phase().clone() else {
                panic!("in review");
            };
            handle
                .advance(ItemPhase::Approved {
                    pr: opened.clone(),
                    reviews,
                })
                .unwrap();
            handle
                .advance(ItemPhase::Done {
                    completion: Completion::Merged { pr: opened, at: at(50) },
                })
                .unwrap();
        }
        state.close_round(&round(1), at(51), RoundEnd::Merged).unwrap();

        let shown = detail(&state);

        assert_eq!(shown.pr, Some(pr(7)));
        assert_eq!(shown.review_passes.len(), 1);
        assert_eq!(shown.item.unwrap().status.to_string(), "done");
        assert_eq!(
            state
                .team(&team_id(1))
                .unwrap()
                .item(item("1.2"))
                .unwrap()
                .phase()
                .kind(),
            PhaseKind::Done
        );
    }

    #[test]
    fn the_members_of_a_round_are_those_with_a_run_that_names_it_in_the_order_of_the_team() {
        let state = in_review();
        let team = state.team(&team_id(1)).unwrap();

        let members: Vec<(Role, String)> = team
            .members_in(&round(1))
            .into_iter()
            .map(|(role, member)| (role, member.task().to_string()))
            .collect();

        assert_eq!(
            members,
            [
                (Role::Coordinator, task(1).to_string()),
                (Role::Member, task(2).to_string())
            ]
        );
        assert_eq!(team.members_in(&round(9)), []);
    }

    #[test]
    fn the_tasks_a_detail_is_given_are_kept_as_they_are() {
        let state = in_review();
        let team = state.team(&team_id(1)).unwrap();
        let (role, member) = team.find(&task(2)).unwrap();
        let tasks = vec![RoundTask {
            task: task(2),
            role,
            agent: member.agent().clone(),
            worktree: "/home/dev/agents/worktrees/team-agents/task-2".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
            runs: member.runs().to_vec(),
        }];

        let shown = RoundDetail::new(team, team.round(&round(1)).unwrap(), tasks.clone());

        assert_eq!(shown.tasks, tasks);
        assert_eq!(shown.tasks[0].runs[0].rounds(), [round(1)]);
    }

    #[test]
    fn a_summary_lists_the_rounds_of_its_team_oldest_first_and_the_events_about_rounds_leave_summaries_as_they_are() {
        let mut state = in_review();
        state
            .open_round(&team_id(1), round(2), Some(item("1.3")), at(60))
            .unwrap();
        let summary = TeamSummary::from(state.team(&team_id(1)).unwrap());
        let mut teams = vec![summary.clone()];

        EventKind::RoundOpened {
            team: team_id(1),
            round: round(3),
            item: None,
        }
        .apply_to(&mut teams);
        EventKind::RoundClosed {
            team: team_id(1),
            round: round(1),
            item: Some(item("1.2")),
            how: RoundEnd::Abandoned,
        }
        .apply_to(&mut teams);

        let ids: Vec<&str> = summary.rounds.iter().map(|found| found.id().as_str()).collect();
        assert_eq!(ids, [round(1).as_str(), round(2).as_str()]);
        assert_eq!(teams, [summary]);
    }
}
