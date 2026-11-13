//! Where a round is, what a verdict leads to, and what waits to be delivered.

use std::num::NonZeroU32;

use agent_core::decide::round::{RoundAction, RoundError, RoundPolicy, RoundState, Stage, Step};
use agent_core::ids::InvestigatorName;
use agent_core::state::{AnsweredVia, ItemPhase, MergeMode, PhaseKind, Question, Team};
use agent_core::status::{ReviewVerdict, TaskStatus};

use super::{Asking, Decision, Exhausted, Next, Unexpected, VerdictError};
use super::{
    after_verdict, asking, decision, exhausted, fix_delivery, items_of, next_of, review_deliveries, round_state,
};
use crate::daemon::round_fixture::{
    LIST, ask, fix, go, item, lead_branch, led, open, pending, pr, ran, ran_in, secs, start, state_of, submit, waits,
};
use crate::fixtures::{at, task};

const AUTOPILOT: RoundPolicy = RoundPolicy {
    merge: MergeMode::Autopilot,
    fix_passes: 5,
};

const NO_MERGE: RoundPolicy = RoundPolicy {
    merge: MergeMode::NoMerge,
    fix_passes: 5,
};

fn reviewer(name: &str) -> InvestigatorName {
    name.parse().unwrap()
}

fn pass(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

/// A team whose item 1.1 is in review on the pull request 7 of the coordinator's branch, with the reviews of
/// `inv-codex` (task 20) and `inv-gemini` (task 21) pending, asked at 101 seconds.
fn in_review() -> Team {
    let mut team = led(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/work-1");
    waits(&mut team, 2);
    open(&mut team, "1.1", 7, &lead_branch());
    ask(
        &mut team,
        "1.1",
        vec![pending("inv-codex", 20, 101), pending("inv-gemini", 21, 101)],
    );
    team
}

/// `team`'s item 1.1, which is in review, goes back for a fix pass at 150 seconds and returns to review with new
/// reviews, as many times as `passes` says.
fn after_passes(mut team: Team, passes: u32) -> Team {
    for round in 0..passes {
        let at_second = 150 + round * 100;
        submit(&mut team, "1.1", 20, ReviewVerdict::RequestChanges, at_second);
        fix(&mut team, "1.1", "inv-opus", &lead_branch(), 7, &lead_branch());
        open(&mut team, "1.1", 7, &lead_branch());
        ask(
            &mut team,
            "1.1",
            vec![
                pending("inv-codex", 20, at_second + 50),
                pending("inv-gemini", 21, at_second + 50),
            ],
        );
    }
    team
}

fn approve_round(team: &mut Team) {
    let (pr_in_review, reviews) = match team.item(item("1.1")).unwrap().phase() {
        ItemPhase::InReview { pr, reviews } => (pr.clone(), reviews.clone()),
        other => panic!("{other:?}"),
    };
    team.item_mut(item("1.1"))
        .unwrap()
        .advance(ItemPhase::Approved {
            pr: pr_in_review,
            reviews,
        })
        .unwrap();
}

// ---- where a round is --------------------------------------------------------------------------------------------

#[test]
fn an_item_with_no_pull_request_or_done_has_no_round() {
    let mut team = led(LIST);
    assert_eq!(round_state(team.item(item("1.1")).unwrap()), None);

    start(&mut team, "1.1", "claude-1", "agents/claude-1/work-1");
    assert_eq!(round_state(team.item(item("1.1")).unwrap()), None);
}

#[test]
fn an_item_in_review_with_no_review_asked_waits_for_reviewers_and_has_had_no_fix_pass() {
    let mut team = led(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/work-1");
    open(&mut team, "1.1", 7, &lead_branch());

    assert_eq!(
        round_state(team.item(item("1.1")).unwrap()),
        Some(RoundState::InReview {
            pr: pr(7, &lead_branch()).number,
            passes: 0,
            approvals: vec![],
        })
    );
}

#[test]
fn the_approvals_of_a_round_are_the_reviewers_that_approved_and_a_pending_review_is_none() {
    let mut team = in_review();
    submit(&mut team, "1.1", 21, ReviewVerdict::Approve, 120);

    assert_eq!(
        round_state(team.item(item("1.1")).unwrap()),
        Some(RoundState::InReview {
            pr: pr(7, &lead_branch()).number,
            passes: 0,
            approvals: vec![reviewer("inv-gemini")],
        })
    );
}

#[test]
fn an_item_in_review_that_a_reviewer_sent_back_waits_for_the_human_with_the_passes_it_has_had() {
    let mut team = after_passes(in_review(), 2);
    submit(&mut team, "1.1", 21, ReviewVerdict::RequestChanges, 400);

    assert_eq!(
        round_state(team.item(item("1.1")).unwrap()),
        Some(RoundState::AskingHuman {
            pr: pr(7, &lead_branch()).number,
            passes: 2,
        })
    );
}

#[test]
fn an_approved_item_is_approved() {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::Approve, 120);
    submit(&mut team, "1.1", 21, ReviewVerdict::Approve, 121);
    approve_round(&mut team);

    assert_eq!(
        round_state(team.item(item("1.1")).unwrap()),
        Some(RoundState::Approved {
            pr: pr(7, &lead_branch()).number
        })
    );
}

#[test]
fn an_item_that_is_being_fixed_is_in_its_fix_pass() {
    let mut team = after_passes(in_review(), 1);
    submit(&mut team, "1.1", 20, ReviewVerdict::RequestChanges, 300);
    fix(&mut team, "1.1", "inv-opus", &lead_branch(), 7, &lead_branch());

    assert_eq!(
        round_state(team.item(item("1.1")).unwrap()),
        Some(RoundState::MemberWork {
            stage: Stage::Fixing {
                pr: pr(7, &lead_branch()).number,
                pass: pass(2),
            }
        })
    );
}

// ---- what a verdict leads to -----------------------------------------------------------------------------------

#[test]
fn the_first_approval_waits_for_the_second() {
    let team = in_review();

    assert_eq!(
        after_verdict(
            team.item(item("1.1")).unwrap(),
            &reviewer("inv-codex"),
            ReviewVerdict::Approve,
            AUTOPILOT
        ),
        Ok(Next::Wait)
    );
}

#[test]
fn the_second_approval_merges_in_autopilot_and_hands_the_pull_request_to_the_human_with_no_merge() {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::Approve, 120);
    let progress = team.item(item("1.1")).unwrap();

    let merge = after_verdict(progress, &reviewer("inv-gemini"), ReviewVerdict::Approve, AUTOPILOT);
    let hand_over = after_verdict(progress, &reviewer("inv-gemini"), ReviewVerdict::Approve, NO_MERGE);

    assert_eq!(merge, Ok(Next::Merge));
    assert_eq!(hand_over, Ok(Next::HandToHuman));
}

#[test]
fn a_request_for_changes_starts_the_next_fix_pass_while_passes_are_left() {
    for (passes, next) in [(0, 1), (1, 2), (4, 5)] {
        let team = after_passes(in_review(), passes);

        let after = after_verdict(
            team.item(item("1.1")).unwrap(),
            &reviewer("inv-codex"),
            ReviewVerdict::RequestChanges,
            AUTOPILOT,
        );

        assert_eq!(after, Ok(Next::FixPass { pass: pass(next) }), "{passes} passes");
    }
}

#[test]
fn a_request_for_changes_asks_the_human_when_the_passes_are_used_up() {
    let team = after_passes(in_review(), 5);

    let after = after_verdict(
        team.item(item("1.1")).unwrap(),
        &reviewer("inv-gemini"),
        ReviewVerdict::RequestChanges,
        AUTOPILOT,
    );

    assert_eq!(after, Ok(Next::AskHuman));
}

#[test]
fn with_no_fix_passes_configured_the_human_is_asked_at_the_first_request_for_changes() {
    let team = in_review();
    let policy = RoundPolicy {
        merge: MergeMode::Autopilot,
        fix_passes: 0,
    };

    let after = after_verdict(
        team.item(item("1.1")).unwrap(),
        &reviewer("inv-codex"),
        ReviewVerdict::RequestChanges,
        policy,
    );

    assert_eq!(after, Ok(Next::AskHuman));
}

#[test]
fn a_reviewer_that_approved_cannot_approve_again() {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::Approve, 120);

    let after = after_verdict(
        team.item(item("1.1")).unwrap(),
        &reviewer("inv-codex"),
        ReviewVerdict::Approve,
        AUTOPILOT,
    );

    assert_eq!(
        after,
        Err(VerdictError::Round(RoundError::DuplicateReview {
            reviewer: reviewer("inv-codex")
        }))
    );
}

#[test]
fn a_verdict_for_an_item_that_is_not_in_review_is_refused_with_its_phase() {
    let mut team = led(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/work-1");

    let after = after_verdict(
        team.item(item("1.1")).unwrap(),
        &reviewer("inv-codex"),
        ReviewVerdict::Approve,
        AUTOPILOT,
    );

    assert_eq!(
        after,
        Err(VerdictError::NotInReview {
            item: item("1.1"),
            phase: PhaseKind::InProgress
        })
    );
}

#[test]
fn only_the_actions_a_verdict_leads_to_are_taken_from_the_round_machine() {
    let step = |action| Step {
        state: RoundState::start(),
        action,
    };
    for (action, next) in [
        (RoundAction::AwaitReviews, Next::Wait),
        (RoundAction::Merge, Next::Merge),
        (RoundAction::HandToHuman, Next::HandToHuman),
        (
            RoundAction::StartFixPass { pass: pass(3) },
            Next::FixPass { pass: pass(3) },
        ),
        (RoundAction::AskHuman, Next::AskHuman),
    ] {
        assert_eq!(next_of(&step(action)), Ok(next), "{action:?}");
    }
    for action in [
        RoundAction::IntegrateBranch,
        RoundAction::FixGate,
        RoundAction::PublishPullRequest,
        RoundAction::RequestReviews,
        RoundAction::CompleteItem,
    ] {
        assert_eq!(next_of(&step(action)), Err(Unexpected { action }), "{action:?}");
    }
}

// ---- the items of a pull request -----------------------------------------------------------------------------

#[test]
fn the_items_of_a_pull_request_are_those_in_review_fixing_or_approved_on_its_number_in_the_order_of_the_list() {
    let mut team = led(LIST);
    for (id, member) in [("1.1", "claude-1"), ("2.1", "claude-2")] {
        start(&mut team, id, member, &format!("agents/{member}/work"));
        open(&mut team, id, 7, &lead_branch());
    }

    assert_eq!(items_of(&team, pr(7, "x").number), [item("1.1"), item("2.1")]);
    assert_eq!(items_of(&team, pr(8, "x").number), []);
}

#[test]
fn an_item_that_is_done_or_has_no_pull_request_is_not_an_item_of_the_pull_request() {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::Approve, 120);
    submit(&mut team, "1.1", 21, ReviewVerdict::Approve, 121);
    approve_round(&mut team);
    team.item_mut(item("1.1"))
        .unwrap()
        .advance(ItemPhase::Done {
            completion: agent_core::state::Completion::Merged {
                pr: pr(7, &lead_branch()),
                at: at(200),
            },
        })
        .unwrap();
    start(&mut team, "2.1", "claude-2", "agents/claude-2/work-2");
    open(&mut team, "2.1", 8, &lead_branch());

    assert_eq!(items_of(&team, pr(7, "x").number), []);
    assert_eq!(items_of(&team, pr(8, "x").number), [item("2.1")]);
}

// ---- the pull requests that wait for the human ----------------------------------------------------------------

#[test]
fn a_pull_request_with_a_request_for_changes_in_its_round_waits_for_the_human_since_the_last_one_came() {
    let mut team = after_passes(in_review(), 2);
    submit(&mut team, "1.1", 21, ReviewVerdict::RequestChanges, 400);

    assert_eq!(
        exhausted(&team),
        [Exhausted {
            pr: pr(7, &lead_branch()),
            items: vec![item("1.1")],
            passes: 2,
            requested_at: secs(400),
        }]
    );
}

#[test]
fn the_time_of_the_request_is_that_of_the_latest_request_for_changes_of_the_round() {
    let mut team = after_passes(in_review(), 1);
    submit(&mut team, "1.1", 20, ReviewVerdict::RequestChanges, 300);
    submit(&mut team, "1.1", 21, ReviewVerdict::RequestChanges, 320);

    let found = exhausted(&team);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].requested_at, secs(320));
}

#[test]
fn a_pull_request_whose_reviews_are_pending_or_approving_or_being_fixed_waits_for_nobody() {
    let mut team = in_review();
    assert_eq!(exhausted(&team), []);
    submit(&mut team, "1.1", 20, ReviewVerdict::Approve, 120);
    assert_eq!(exhausted(&team), []);

    let mut fixing = in_review();
    submit(&mut fixing, "1.1", 20, ReviewVerdict::RequestChanges, 150);
    fix(&mut fixing, "1.1", "inv-opus", &lead_branch(), 7, &lead_branch());
    assert_eq!(exhausted(&fixing), []);
}

// ---- what the human said --------------------------------------------------------------------------------------

/// The lead of a team with the question of round 1 asked at `asked` seconds, answered at `answered` when it is given.
fn lead_with_question(asked: u32, answered: Option<u32>) -> Team {
    let mut team = led(LIST);
    let (_, mut lead) = team.find_mut(&task(1)).unwrap();
    let file = agent_core::config::AbsolutePath::try_from("/home/dev/agents/tasks/questions-1.md".to_owned()).unwrap();
    lead.ask_question(Question::new(NonZeroU32::MIN, file, at(asked)))
        .unwrap();
    if let Some(answered) = answered {
        lead.answer_question(NonZeroU32::MIN, at(answered), AnsweredVia::Command)
            .unwrap();
    }
    team
}

#[test]
fn with_no_question_since_the_request_the_human_is_still_to_be_asked() {
    let team = led(LIST);

    assert_eq!(decision(team.crew().coordinator().unwrap(), secs(400)), Decision::Ask);
}

#[test]
fn a_question_from_before_the_request_is_not_the_one_about_it() {
    let team = lead_with_question(300, Some(310));

    assert_eq!(decision(team.crew().coordinator().unwrap(), secs(400)), Decision::Ask);
}

#[test]
fn a_question_put_since_the_request_is_waiting_or_answered() {
    let open = lead_with_question(420, None);
    let answered = lead_with_question(420, Some(500));

    assert_eq!(
        decision(open.crew().coordinator().unwrap(), secs(400)),
        Decision::Waiting
    );
    assert_eq!(
        decision(answered.crew().coordinator().unwrap(), secs(400)),
        Decision::Answered
    );
}

#[test]
fn a_question_put_at_the_very_second_of_the_request_is_about_it() {
    let team = lead_with_question(400, None);

    assert_eq!(
        decision(team.crew().coordinator().unwrap(), secs(400)),
        Decision::Waiting
    );
}

// ---- what waits to be delivered -------------------------------------------------------------------------------

/// A team with item 1.1 being fixed since the request for changes at 150 seconds, led by a coordinator that ran from
/// `started` to `started + 10` seconds and waits.
fn fixing_with_lead_run_at(started: u32) -> Team {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::RequestChanges, 150);
    fix(&mut team, "1.1", "inv-opus", &lead_branch(), 7, &lead_branch());
    ran(&mut team, 1, 1, started, started + 10);
    team
}

#[test]
fn a_fix_pass_waits_to_be_given_to_a_lead_that_waits_and_has_not_run_since_the_request() {
    let team = fixing_with_lead_run_at(50);

    assert_eq!(fix_delivery(&team), Some(task(1)));
}

#[test]
fn a_fix_pass_is_given_once_the_lead_started_a_run_after_the_request() {
    let team = fixing_with_lead_run_at(160);

    assert_eq!(fix_delivery(&team), None);
}

#[test]
fn a_fix_pass_waits_for_a_lead_that_is_at_work() {
    let mut team = fixing_with_lead_run_at(50);
    go(&mut team, 1, &[TaskStatus::Queued, TaskStatus::Running]);

    assert_eq!(fix_delivery(&team), None);
}

#[test]
fn a_lead_that_never_ran_is_given_the_fix_pass_when_it_waits() {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::RequestChanges, 150);
    fix(&mut team, "1.1", "inv-opus", &lead_branch(), 7, &lead_branch());
    waits(&mut team, 1);

    assert_eq!(fix_delivery(&team), Some(task(1)));
}

#[test]
fn no_fix_pass_waits_when_no_item_is_being_fixed() {
    let mut team = in_review();
    waits(&mut team, 1);

    assert_eq!(fix_delivery(&team), None);
}

/// A state with the team of [`in_review`] and its two reviewers.
fn reviewed() -> agent_core::state::State {
    state_of(in_review(), &[("inv-codex", 20), ("inv-gemini", 21)])
}

#[test]
fn a_review_waits_to_be_asked_of_a_reviewer_that_waits_and_has_not_run_since_it_was_asked() {
    let mut state = reviewed();
    ran_in(&mut state, 20, 20, 50, 60);

    let team = state.team(&crate::fixtures::team_id(0)).unwrap();

    assert_eq!(review_deliveries(team), [task(20)]);
}

#[test]
fn a_reviewer_that_has_not_run_yet_starts_by_itself_and_is_not_asked_again() {
    let state = reviewed();
    let team = state.team(&crate::fixtures::team_id(0)).unwrap();

    assert_eq!(review_deliveries(team), []);
}

#[test]
fn a_reviewer_that_started_a_run_after_the_review_was_asked_is_not_asked_again() {
    let mut state = reviewed();
    ran_in(&mut state, 20, 20, 110, 120);
    let team = state.team(&crate::fixtures::team_id(0)).unwrap();

    assert_eq!(review_deliveries(team), []);
}

#[test]
fn a_reviewer_that_is_at_work_is_asked_when_it_waits() {
    let mut state = reviewed();
    ran_in(&mut state, 20, 20, 50, 60);
    ran_in(&mut state, 21, 21, 50, 60);
    {
        let (_, mut found) = state.find_task_mut(&task(21)).unwrap();
        found.transition(TaskStatus::Queued).unwrap();
        found.transition(TaskStatus::Running).unwrap();
    }
    let team = state.team(&crate::fixtures::team_id(0)).unwrap();

    assert_eq!(review_deliveries(team), [task(20)]);
}

#[test]
fn a_reviewer_that_gave_its_review_is_not_asked() {
    let mut team = in_review();
    submit(&mut team, "1.1", 20, ReviewVerdict::Approve, 120);
    let mut state = state_of(team, &[("inv-codex", 20), ("inv-gemini", 21)]);
    ran_in(&mut state, 20, 20, 50, 60);
    let team = state.team(&crate::fixtures::team_id(0)).unwrap();

    assert_eq!(review_deliveries(team), []);
}

// ---- asking the human -----------------------------------------------------------------------------------------

/// A team in which a reviewer sent item 1.1 back after the passes were used up, at 400 seconds, led by a coordinator
/// that ran from `started` seconds.
fn exhausted_with_lead_run_at(started: u32) -> Team {
    let mut team = after_passes(in_review(), 2);
    submit(&mut team, "1.1", 21, ReviewVerdict::RequestChanges, 400);
    ran(&mut team, 1, 1, started, started + 10);
    team
}

fn found(team: &Team) -> Exhausted {
    exhausted(team).into_iter().next().unwrap()
}

#[test]
fn the_lead_is_woken_to_ask_the_human_when_it_waits_and_has_not_run_since_the_request() {
    let team = exhausted_with_lead_run_at(50);

    assert_eq!(
        asking(&team),
        [Asking::Wake {
            lead: task(1),
            exhausted: found(&team)
        }]
    );
}

#[test]
fn the_human_is_told_directly_when_the_lead_ran_since_the_request_and_put_no_question() {
    let team = exhausted_with_lead_run_at(410);

    assert_eq!(
        asking(&team),
        [Asking::Notify {
            exhausted: found(&team)
        }]
    );
}

#[test]
fn nothing_is_done_while_the_question_is_open_or_answered_or_the_lead_is_at_work() {
    let mut open = exhausted_with_lead_run_at(50);
    {
        let (_, mut lead) = open.find_mut(&task(1)).unwrap();
        let file = agent_core::config::AbsolutePath::try_from("/tmp/q.md".to_owned()).unwrap();
        lead.ask_question(Question::new(NonZeroU32::MIN, file, at(420)))
            .unwrap();
    }
    let mut at_work = exhausted_with_lead_run_at(50);
    go(&mut at_work, 1, &[TaskStatus::Queued, TaskStatus::Running]);

    assert_eq!(asking(&open), []);
    assert_eq!(asking(&at_work), []);
}
