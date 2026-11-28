//! A hand-over through the state: only to an agent outside the team, of a task that is not in a run, and so that the
//! state it leaves loads again.

use crate::ids::{AgentName, InvestigatorName, TaskId};
use crate::kinds::Role;
use crate::state::{Choice, HandOverError, Holding, Participant, State, Timestamp};
use crate::status::TaskStatus;

/// The failed member's task of the 1.1 fixture: `codex-1`, a member of a team led by `inv-opus`.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// The state of 1.1, written by main.
fn old_state() -> State {
    State::from_json(include_str!("../../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When the hand-overs of these tests happen.
fn at() -> Timestamp {
    "2026-11-21T00:00:00Z".parse().unwrap()
}

fn normal(name: &str) -> Participant {
    Participant::Normal(name.parse::<AgentName>().unwrap())
}

fn investigator(name: &str) -> Participant {
    Participant::Investigator(name.parse::<InvestigatorName>().unwrap())
}

#[test]
fn a_task_handed_over_to_an_agent_outside_the_team_is_held_by_it_and_the_state_loads_again() {
    let mut state = old_state();

    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.agent(), &normal("claude-9"));
    assert_eq!(
        member.past_holders(),
        vec![Holding {
            agent: normal("codex-1"),
            until: at(),
        }]
    );
}

#[test]
fn a_task_is_not_handed_over_to_the_agent_that_holds_it() {
    let mut state = old_state();

    let refused = state.hand_over_task(&task(), normal("codex-1"), at());

    assert_eq!(refused, Err(HandOverError::SameHolder { task: task() }));
}

#[test]
fn a_task_is_not_handed_over_to_an_agent_of_its_own_team() {
    for agent in ["inv-opus", "inv-codex"] {
        let mut state = old_state();

        let refused = state.hand_over_task(&task(), investigator(agent), at());

        assert_eq!(
            refused,
            Err(HandOverError::InTeam {
                task: task(),
                agent: agent.to_owned(),
            }),
            "{agent} leads or reviews the team"
        );
    }
}

#[test]
fn a_task_in_the_middle_of_a_run_is_not_handed_over() {
    let mut state = old_state();
    {
        let (_, mut member) = state.find_task_mut(&task()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run("run-20261121-000000-a1a1".parse().unwrap(), at(), None)
            .unwrap();
    }

    let refused = state.hand_over_task(&task(), normal("claude-9"), at());

    assert_eq!(refused, Err(HandOverError::InRun { task: task() }));
}

#[test]
fn a_task_that_is_not_in_the_state_or_is_a_workers_or_a_reviewers_is_not_handed_over() {
    let mut state = old_state();
    let unknown: TaskId = "task-20261121-000000-zzzz".parse().unwrap();
    let worker: TaskId = "task-20261111-193003-k3x9".parse().unwrap();
    let reviewer: TaskId = "task-20261111-193007-k3x9".parse().unwrap();

    assert_eq!(
        state.hand_over_task(&unknown, normal("claude-9"), at()),
        Err(HandOverError::NoTask { task: unknown })
    );
    assert_eq!(
        state.hand_over_task(&worker, normal("claude-9"), at()),
        Err(HandOverError::Role {
            task: worker,
            role: Role::Worker,
        })
    );
    assert_eq!(
        state.hand_over_task(&reviewer, investigator("inv-kimi"), at()),
        Err(HandOverError::Role {
            task: reviewer,
            role: Role::Reviewer,
        })
    );
}

#[test]
fn a_task_is_not_handed_over_to_an_agent_of_a_lower_tier_and_the_state_still_loads() {
    let coordinator: TaskId = "task-20261111-193001-k3x9".parse().unwrap();
    let cases = [
        (
            task(),
            Participant::Worker("worker-9".parse::<AgentName>().unwrap()),
            crate::kinds::Tier::Worker,
            crate::kinds::Tier::Normal,
        ),
        (
            coordinator,
            normal("claude-9"),
            crate::kinds::Tier::Normal,
            crate::kinds::Tier::Investigator,
        ),
    ];
    for (task, holder, lower, held) in cases {
        let mut state = old_state();

        let refused = state.hand_over_task(&task, holder, at());

        assert_eq!(
            refused,
            Err(HandOverError::LowerTier {
                task: task.clone(),
                holder: lower,
                held,
            })
        );
        State::from_json(&state.to_json().unwrap()).expect("the refused hand-over leaves a state that loads");
    }
}

#[test]
fn a_members_task_may_be_handed_over_to_an_investigator_and_the_state_loads_again() {
    let mut state = old_state();

    state.hand_over_task(&task(), investigator("inv-kimi"), at()).unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    assert_eq!(
        loaded.find_task(&task()).unwrap().member.agent(),
        &investigator("inv-kimi")
    );
}

/// The state of the 1.1 fixture with its failed member handed over to `claude-9` and its session begun for it.
fn handed_over_and_begun() -> State {
    let mut state = old_state();
    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();
    let begun = state
        .begin_holder_session(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();
    assert!(begun, "a task with runs begins a session for its new holder");
    state
}

#[test]
fn a_task_handed_over_resumes_no_session_of_the_agent_before_and_its_next_run_is_briefed() {
    let state = handed_over_and_begun();

    let member = state.find_task(&task()).unwrap().member;
    assert_eq!(
        member.last_session(),
        None,
        "no session of codex-1 is resumed by claude-9"
    );
    assert_eq!(member.session_runs().len(), 0);
    assert_eq!(
        member.bad_runs_since_renewal(),
        0,
        "the bad runs of codex-1 are not counted against claude-9"
    );
    assert!(member.awaits_new_session(), "claude-9's first run is briefed");
    let renewals = member.renewals().expect("the session boundary is recorded");
    assert_eq!(renewals.reason(), crate::decide::ladder::Trouble::NoStatus);
}

#[test]
fn a_hand_over_is_not_counted_as_a_new_session_of_the_agent() {
    let mut state = old_state();
    state
        .renew_task(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();
    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();

    state
        .begin_holder_session(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    let renewals = *loaded.find_task(&task()).unwrap().member.renewals().unwrap();
    assert_eq!(renewals.used(), 1, "the one new session of codex-1, not the hand-over");
    let fresh = handed_over_and_begun();
    let renewals = fresh.find_task(&task()).unwrap().member.renewals().copied().unwrap();
    assert_eq!(renewals.used(), 0, "a hand-over alone counts no new session");
}

#[test]
fn a_session_is_begun_only_for_a_task_in_the_state_that_is_not_a_workers_or_a_reviewers() {
    let mut state = old_state();
    let unknown: TaskId = "task-20261121-000000-zzzz".parse().unwrap();
    let worker: TaskId = "task-20261111-193003-k3x9".parse().unwrap();

    let trouble = crate::decide::ladder::Trouble::Hang;
    assert!(matches!(
        state.begin_holder_session(&unknown, trouble, at()),
        Err(crate::state::RenewTaskError::NoTask { task }) if task == unknown
    ));
    assert!(matches!(
        state.begin_holder_session(&worker, trouble, at()),
        Err(crate::state::RenewTaskError::Role { task, role: Role::Worker }) if task == worker
    ));
}

#[test]
fn a_task_is_known_on_its_teams_bus_by_the_agent_it_was_first_given_to() {
    let mut state = old_state();
    assert_eq!(state.find_task(&task()).unwrap().member.bus_agent(), &normal("codex-1"));

    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();
    state.hand_over_task(&task(), investigator("inv-kimi"), at()).unwrap();

    let member = state.find_task(&task()).unwrap().member;
    assert_eq!(member.agent(), &investigator("inv-kimi"));
    assert_eq!(
        member.bus_agent(),
        &normal("codex-1"),
        "the bus keeps the name of the first holder"
    );
}

#[test]
fn a_task_whose_last_step_went_bad_keeps_the_question_put_to_the_human_and_resumes_no_session() {
    let mut state = handed_over_and_begun();
    let question = std::num::NonZeroU32::new(3).unwrap();

    let begun = state
        .ask_after_last_step(&task(), crate::decide::ladder::Trouble::Hang, question, at())
        .unwrap();

    assert!(
        begun,
        "a task with runs begins a session for whatever the human answers"
    );
    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.asked_after_last_step(), Some(question));
    assert_eq!(member.last_session(), None, "no session of the runs before is resumed");
    assert_eq!(
        member.renewals().map(crate::state::Renewals::used),
        Some(0),
        "no new session is counted"
    );
    assert_eq!(
        member.renewals().map(crate::state::Renewals::reason),
        Some(crate::decide::ladder::Trouble::Hang)
    );
}

#[test]
fn a_task_never_asked_after_its_last_step_has_no_such_question() {
    let state = handed_over_and_begun();

    assert_eq!(state.find_task(&task()).unwrap().member.asked_after_last_step(), None);
    assert_eq!(
        old_state().find_task(&task()).unwrap().member.asked_after_last_step(),
        None
    );
}

#[test]
fn a_workers_task_is_never_asked_about_after_its_last_step() {
    let mut state = old_state();
    let worker: TaskId = "task-20261111-193003-k3x9".parse().unwrap();
    let question = std::num::NonZeroU32::new(1).unwrap();

    let refused = state.ask_after_last_step(&worker, crate::decide::ladder::Trouble::NoStatus, question, at());

    assert!(matches!(
        refused,
        Err(crate::state::RenewTaskError::Role { task, role: Role::Worker }) if task == worker
    ));
}

#[test]
fn a_members_work_owed_to_its_lead_is_kept_until_the_lead_is_told() {
    let mut state = old_state();

    state
        .owe_lead(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    let member = loaded.find_task(&task()).unwrap().member;
    assert!(member.owes_lead(), "the work is owed to the lead across a restart");
    assert_eq!(
        member.renewals().map(crate::state::Renewals::used),
        Some(0),
        "no new session is counted"
    );
    assert_eq!(
        member.renewals().map(crate::state::Renewals::reason),
        Some(crate::decide::ladder::Trouble::NoStatus)
    );

    let mut state = loaded;
    state.lead_told(&task()).unwrap();
    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    assert!(
        !loaded.find_task(&task()).unwrap().member.owes_lead(),
        "told once, owed no more"
    );
}

#[test]
fn a_member_given_to_its_lead_is_remembered_to_be_so_after_the_lead_is_told() {
    let mut state = old_state();
    assert!(
        !state.find_task(&task()).unwrap().member.given_to_lead(),
        "a state of main gave nothing to a lead"
    );

    state
        .owe_lead(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();
    state.lead_told(&task()).unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    assert!(
        loaded.find_task(&task()).unwrap().member.given_to_lead(),
        "the hand-over to the lead stays recorded once the lead is told, across a restart"
    );
}

#[test]
fn a_member_given_to_its_lead_is_not_so_any_more_once_it_runs_again() {
    let mut state = old_state();
    state
        .owe_lead(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();
    state.lead_told(&task()).unwrap();
    assert!(state.find_task(&task()).unwrap().member.given_to_lead());

    {
        let (_, mut member) = state.find_task_mut(&task()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run(
                "run-20261121-010000-b2b2".parse().unwrap(),
                "2026-11-21T01:00:00Z".parse().unwrap(),
                None,
            )
            .unwrap();
    }

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    assert!(
        !loaded.find_task(&task()).unwrap().member.given_to_lead(),
        "work done after the hand-over is not the lead's: the member is looked at again"
    );
}

#[test]
fn a_member_started_in_a_new_session_was_not_given_to_its_lead() {
    let mut state = old_state();

    state
        .renew_task(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();

    assert!(!state.find_task(&task()).unwrap().member.given_to_lead());
}

#[test]
fn only_a_members_work_is_owed_to_a_lead() {
    let mut state = old_state();
    assert!(
        !state.find_task(&task()).unwrap().member.owes_lead(),
        "a state of main owes nothing"
    );
    let coordinator: TaskId = "task-20261111-193001-k3x9".parse().unwrap();
    let worker: TaskId = "task-20261111-193003-k3x9".parse().unwrap();

    for (other, role) in [(coordinator, Role::Coordinator), (worker, Role::Worker)] {
        let refused = state.owe_lead(&other, crate::decide::ladder::Trouble::Hang, at());
        assert!(
            matches!(&refused, Err(crate::state::RenewTaskError::Role { task, role: refused }) if *task == other && *refused == role),
            "{refused:?}"
        );
    }
}

#[test]
fn handing_a_task_to_a_split_replacement_does_not_resume_the_previous_holders_session() {
    let mut state = old_state();
    assert!(state.find_task(&task()).unwrap().member.last_session().is_some());

    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).unwrap();
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.last_session(), None);
    assert_eq!(member.session_runs().len(), 0);
    assert_eq!(member.bad_runs_since_renewal(), 0);
    assert_eq!(member.renewals(), None);
}

#[test]
fn replacement_run_keeps_earlier_history_but_only_resumes_its_own_session() {
    let mut state = old_state();
    let earlier = state.find_task(&task()).unwrap().member.runs().len();
    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();
    let session = "replacement-session".parse().unwrap();
    let (_, mut member) = state.find_task_mut(&task()).unwrap();
    member
        .begin_run("run-20261121-000000-frsh".parse().unwrap(), at(), Some(session))
        .unwrap();
    let loaded = State::from_json(&state.to_json().unwrap()).unwrap();
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.runs().len(), earlier + 1);
    assert_eq!(member.session_runs().len(), 1);
    assert_eq!(
        member.last_session().map(crate::state::SessionId::as_str),
        Some("replacement-session")
    );
}

#[test]
fn handing_owed_work_to_an_outside_replacement_clears_the_prior_holders_lead_debt() {
    let mut state = old_state();
    state
        .owe_lead(&task(), crate::decide::ladder::Trouble::NoStatus, at())
        .unwrap();
    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();
    let loaded = State::from_json(&state.to_json().unwrap()).unwrap();
    let member = loaded.find_task(&task()).unwrap().member;
    assert!(
        !member.owes_lead(),
        "a replacement owns the work instead of leaving it owed to the lead"
    );
    assert_eq!(member.renewals().map(crate::state::Renewals::used), Some(0));
}

#[test]
fn human_swap_to_lower_tier_is_applied() {
    let mut state = old_state();
    let coordinator: TaskId = "task-20261111-193001-k3x9".parse().unwrap();

    assert_eq!(
        state.hand_over_task_as(&coordinator, normal("claude-9"), Choice::Human, at()),
        Ok(())
    );

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    let member = loaded.find_task(&coordinator).unwrap().member;
    assert_eq!(member.agent(), &normal("claude-9"));
}

#[test]
fn harness_handover_to_lower_tier_is_still_refused() {
    let mut state = old_state();
    let coordinator: TaskId = "task-20261111-193001-k3x9".parse().unwrap();

    let refused = state.hand_over_task_as(&coordinator, normal("claude-9"), Choice::Harness, at());

    assert_eq!(
        refused,
        Err(HandOverError::LowerTier {
            task: coordinator,
            holder: crate::kinds::Tier::Normal,
            held: crate::kinds::Tier::Investigator,
        })
    );
}

#[test]
fn human_swap_same_holder_refused() {
    let mut state = old_state();

    let refused = state.hand_over_task_as(&task(), normal("codex-1"), Choice::Human, at());

    assert_eq!(refused, Err(HandOverError::SameHolder { task: task() }));
}

#[test]
fn human_swap_in_team_refused() {
    let mut state = old_state();

    let refused = state.hand_over_task_as(&task(), investigator("inv-opus"), Choice::Human, at());

    assert_eq!(
        refused,
        Err(HandOverError::InTeam {
            task: task(),
            agent: "inv-opus".to_owned(),
        })
    );
}

#[test]
fn human_swap_in_run_refused() {
    let mut state = old_state();
    {
        let (_, mut member) = state.find_task_mut(&task()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run("run-20261121-000000-a1a1".parse().unwrap(), at(), None)
            .unwrap();
    }

    let refused = state.hand_over_task_as(&task(), normal("claude-9"), Choice::Human, at());

    assert_eq!(refused, Err(HandOverError::InRun { task: task() }));
}

#[test]
fn human_swap_of_reviewer_task_refused() {
    let mut state = old_state();
    let reviewer: TaskId = "task-20261111-193007-k3x9".parse().unwrap();

    let refused = state.hand_over_task_as(&reviewer, investigator("inv-kimi"), Choice::Human, at());

    assert_eq!(
        refused,
        Err(HandOverError::Role {
            task: reviewer,
            role: Role::Reviewer,
        })
    );
}

#[test]
fn state_with_reason_swap_reads() {
    let mut state = old_state();
    state
        .begin_holder_session(&task(), crate::decide::ladder::Trouble::Swap, at())
        .expect("holder session begins with reason swap");

    let member = state.find_task(&task()).expect("the task").member;
    assert_eq!(
        member.renewals().map(crate::state::Renewals::reason),
        Some(crate::decide::ladder::Trouble::Swap)
    );

    let saved = state.to_json().expect("state serializes to json");
    assert!(saved.contains("\"reason\": \"swap\""));

    let loaded = State::from_json(&saved).expect("state with reason swap reads back");
    assert_eq!(
        loaded
            .find_task(&task())
            .expect("the task")
            .member
            .renewals()
            .map(crate::state::Renewals::reason),
        Some(crate::decide::ladder::Trouble::Swap)
    );
    assert_eq!(loaded, state);
}

#[test]
fn state_of_today_reads_as_before() {
    let before = State::from_json(include_str!("../../../tests/fixtures/state-before-swap.json"))
        .expect("state before swap reads");
    assert!(before.find_task(&task()).is_some());
}

#[test]
fn human_swap_to_worker_is_refused_without_changing_state() {
    for task in [
        task(),
        "task-20261111-193001-k3x9".parse().unwrap(),
        "task-20261111-193011-k3x9".parse().unwrap(),
    ] {
        let mut state = old_state();
        let before = state.to_json().unwrap();
        let worker: AgentName = "worker-9".parse().unwrap();

        let refused = state.hand_over_task_as(&task, Participant::Worker(worker.clone()), Choice::Human, at());

        assert_eq!(refused, Err(HandOverError::Worker { task, agent: worker }));
        assert_eq!(state.to_json().unwrap(), before);
        State::from_json(&before).expect("the unchanged state loads again");
    }
}

#[test]
fn human_swaps_preserve_the_harness_budget_across_reload() {
    let mut state = old_state();
    state
        .hand_over_task_as(&task(), normal("claude-9"), Choice::Human, at())
        .unwrap();
    let loaded = State::from_json(&state.to_json().unwrap()).unwrap();
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.harness_takeovers(), 0);
    state.hand_over_task(&task(), normal("claude-8"), at()).unwrap();
    let loaded = State::from_json(&state.to_json().unwrap()).unwrap();
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.past_holders().len(), 2);
    assert_eq!(member.harness_takeovers(), 1);
}

#[test]
fn member_without_human_handovers_reads_as_zero() {
    let member: crate::state::Member = serde_json::from_str(include_str!(
        "../../../tests/fixtures/member-before-human-handovers.json"
    ))
    .unwrap();
    let written = serde_json::to_value(&member).unwrap();
    assert_eq!(written.get("human_handovers"), None);
    assert_eq!(member.harness_takeovers(), member.past_holders().len());
}

#[test]
fn members_without_human_handovers_keep_their_persisted_shape() {
    let state = old_state();
    let written = state.to_json().unwrap();
    assert!(!written.contains("\"human_handovers\""));
}

#[test]
fn human_handovers_cannot_exceed_the_persisted_holder_history() {
    let mut wire: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/member-before-human-handovers.json"
    ))
    .unwrap();
    wire["human_handovers"] = serde_json::json!(1);
    let error = serde_json::from_value::<crate::state::Member>(wire).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
}

#[test]
fn human_handovers_accept_both_holder_history_boundaries() {
    let mut state = old_state();
    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();
    let mut wire = serde_json::to_value(state.find_task(&task()).unwrap().member).unwrap();
    for count in [0, 1] {
        wire["human_handovers"] = serde_json::json!(count);
        let loaded: crate::state::Member = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(loaded.harness_takeovers(), 1 - count);
    }
    wire["human_handovers"] = serde_json::json!(2);
    let error = serde_json::from_value::<crate::state::Member>(wire).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
}
