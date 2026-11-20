use super::*;
use crate::config::{Config, Plan as Policy};
use crate::ids::ItemId;
use crate::ids::TaskId;
use crate::kinds::Role;
use crate::plan::{Plan, parse_plan};
use crate::state::{Member, Participant, PlanKeeping, PlanReason, PlanStage, RunEnd};
use crate::status::RunOutcome;

/// The end of a run of the agent that reports `outcome`.
fn ended(outcome: RunOutcome) -> RunEnd {
    RunEnd::Reported { outcome }
}

/// What the reader says of `text`: the plan, or why it does not parse.
fn read(text: &str) -> PlanRead {
    match parse_plan(text) {
        Ok(plan) => PlanRead::Parsed(plan),
        Err(error) => PlanRead::unreadable(&error),
    }
}

/// The task of the members the tests make.
fn task(n: u32) -> TaskId {
    format!("task-20261111-1530{n:02}-k3x9").parse().expect("a task")
}

/// The participant of the members the tests make.
fn normal(name: &str) -> Participant {
    Participant::Normal(name.parse().expect("an agent"))
}

/// The plan of `text`, which must parse.
fn plan(text: &str) -> Plan {
    parse_plan(text).expect("the plan parses")
}

/// An item ID.
fn item(text: &str) -> ItemId {
    text.parse().expect("an item")
}

/// A plan with a step done and a step given up: nothing is left.
const DONE: &str = "1. Work\n[x] 1.1 Write it\n    Check: reviewer\n[-] 1.2 Skip it\n    Not needed.\n";
/// A plan with a step in hand and one still to do.
const IN_HAND: &str = "1. Work\n[>] 1.1 Write it\n[ ] 1.2 Check it\n";
/// A plan with a step whose mark is not one of the four.
const BAD_MARK: &str = "1. Work\n[!] 1.1 Write it\n";

/// What a task keeps after `sent_back` send-backs, in `stage`.
fn keeping(sent_back: u32, stage: PlanStage) -> PlanKeeping {
    let mut member = Member::new(task(1), normal("codex-1"));
    for _ in 0..sent_back {
        member.send_back(PlanReason::Missing);
    }
    if let PlanStage::Asked { question } = stage {
        member.plan_asked(question);
    }
    if stage == PlanStage::Released {
        member.release_plan();
    }
    member.plan().cloned().expect("the member keeps its plan")
}

/// The plan roles of a file without `[plan]`.
fn policy() -> Policy {
    Policy::default()
}

/// The policy of the `[plan]` table `table`, read as the configuration reads it.
fn policy_of(table: &str) -> Policy {
    let config = Config::from_toml_str(table).expect("the plan table is valid");
    config.plan().clone()
}

#[test]
fn a_run_that_reports_done_with_a_clean_plan_goes_on() {
    let end = ended(RunOutcome::Done);
    let read = read(DONE);
    let previous = plan("1. Work\n[ ] 1.1 Write it\n[-] 1.2 Skip it\n    Not needed.\n");
    let fixing = keeping(0, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &read,
        earlier: std::slice::from_ref(&previous),
        keeping: Some(&fixing),
        policy: &policy(),
    });

    assert_eq!(verdict, PlanVerdict::Accept);
}

#[test]
fn a_run_that_ends_with_another_status_is_never_sent_back_for_its_plan() {
    let missing = PlanRead::Missing;
    let keeping = keeping(0, PlanStage::Open);

    for end in [
        ended(RunOutcome::Questions),
        ended(RunOutcome::Failed),
        ended(RunOutcome::Sleeping),
        ended(RunOutcome::Excluded),
        RunEnd::Hung,
        RunEnd::Stopped,
        RunEnd::TurnLimit,
        RunEnd::RateLimited,
    ] {
        let verdict = decide_plan(PlanFacts {
            role: Role::Member,
            end: &end,
            read: &missing,
            earlier: &[],
            keeping: Some(&keeping),
            policy: &policy(),
        });

        assert_eq!(verdict, PlanVerdict::Accept, "{end:?}");
    }
}

#[test]
fn a_role_outside_the_plan_roles_is_never_sent_back() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let keeping = keeping(0, PlanStage::Open);

    for role in [Role::Worker, Role::Reviewer] {
        let verdict = decide_plan(PlanFacts {
            role,
            end: &end,
            read: &missing,
            earlier: &[],
            keeping: Some(&keeping),
            policy: &policy(),
        });

        assert_eq!(verdict, PlanVerdict::Accept, "{role}");
    }
}

#[test]
fn a_task_created_before_plans_is_never_sent_back() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;

    let verdict = decide_plan(PlanFacts {
        role: Role::Solo,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: None,
        policy: &policy(),
    });

    assert_eq!(verdict, PlanVerdict::Accept);
}

#[test]
fn a_limit_of_zero_never_sends_a_run_back() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let keeping = keeping(0, PlanStage::Open);
    let policy = policy_of("[plan]\nbounces = 0\n");

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&keeping),
        policy: &policy,
    });

    assert_eq!(verdict, PlanVerdict::Accept);
}

#[test]
fn a_released_plan_is_never_sent_back_and_a_clean_one_is_accepted() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let keeping = keeping(5, PlanStage::Released);

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&keeping),
        policy: &policy(),
    });

    assert_eq!(verdict, PlanVerdict::Accept);
}

#[test]
fn each_fault_of_a_plan_goes_back_with_its_own_reason() {
    let end = ended(RunOutcome::Done);
    let keeping = keeping(0, PlanStage::Open);
    let before = plan("1. Work\n[x] 1.1 Write it\n[ ] 1.2 Check it\n");
    let after = read("1. Work\n[x] 1.1 Write it\n");
    let unfinished = read(IN_HAND);
    let unreadable = read(BAD_MARK);

    let cases: [(PlanRead, Option<Plan>, PlanReason); 4] = [
        (PlanRead::Missing, None, PlanReason::Missing),
        (
            unreadable,
            None,
            PlanReason::Unreadable {
                line: Some(2.try_into().expect("a line")),
                detail: "line 2: malformed step line".to_owned(),
            },
        ),
        (
            unfinished,
            None,
            PlanReason::Unfinished {
                steps: vec![item("1.1"), item("1.2")],
            },
        ),
        (
            after,
            Some(before),
            PlanReason::LostStep {
                steps: vec![item("1.2")],
            },
        ),
    ];

    for (read, previous, reason) in cases {
        let verdict = decide_plan(PlanFacts {
            role: Role::Member,
            end: &end,
            read: &read,
            earlier: previous.as_slice(),
            keeping: Some(&keeping),
            policy: &policy(),
        });

        assert_eq!(verdict, PlanVerdict::SendBack(reason), "{read:?}");
    }
}

#[test]
fn an_unfinished_plan_names_every_step_to_do_or_in_hand_in_its_order() {
    let end = ended(RunOutcome::Done);
    let read = read("1. Work\n[x] 1.1 Done\n[ ] 1.2 To do\n2. More\n[>] 2.1 In hand\n[ ] 2.2 Later\n");
    let keeping = keeping(0, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &read,
        earlier: &[],
        keeping: Some(&keeping),
        policy: &policy(),
    });

    let steps = vec![item("1.2"), item("2.1"), item("2.2")];
    assert_eq!(verdict, PlanVerdict::SendBack(PlanReason::Unfinished { steps }));
}

#[test]
fn an_unreadable_plan_with_a_fault_of_the_whole_text_names_no_line() {
    let end = ended(RunOutcome::Done);
    let empty = parse_plan("  \n\n").expect_err("the plan is empty");
    let read = PlanRead::unreadable(&empty);
    let keeping = keeping(0, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &read,
        earlier: &[],
        keeping: Some(&keeping),
        policy: &policy(),
    });

    assert_eq!(
        verdict,
        PlanVerdict::SendBack(PlanReason::Unreadable {
            line: None,
            detail: "the plan is empty".to_owned(),
        })
    );
}

#[test]
fn a_lost_step_is_reported_before_a_step_still_to_do() {
    let end = ended(RunOutcome::Done);
    let previous = plan("1. Work\n[x] 1.1 Write it\n[-] 1.2 Skip it\n    Not needed.\n");
    let now = read("1. Work\n[ ] 1.1 Write it\n[>] 1.3 Check it\n");
    let keeping = keeping(0, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &now,
        earlier: std::slice::from_ref(&previous),
        keeping: Some(&keeping),
        policy: &policy(),
    });

    assert_eq!(
        verdict,
        PlanVerdict::SendBack(PlanReason::LostStep {
            steps: vec![item("1.2")]
        })
    );
}

#[test]
fn after_the_limit_the_human_is_asked_once_and_not_again() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let open = keeping(3, PlanStage::Open);
    let asked = keeping(
        3,
        PlanStage::Asked {
            question: 2.try_into().expect("a number"),
        },
    );

    let first = decide_plan(PlanFacts {
        role: Role::Coordinator,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&open),
        policy: &policy(),
    });
    let again = decide_plan(PlanFacts {
        role: Role::Coordinator,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&asked),
        policy: &policy(),
    });

    assert_eq!(first, PlanVerdict::AskHuman(PlanReason::Missing));
    assert_eq!(again, PlanVerdict::AskHuman(PlanReason::Missing));
}

#[test]
fn a_send_back_the_human_granted_raises_the_limit() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let mut member = Member::new(task(1), normal("codex-1"));
    member.send_back(PlanReason::Missing);
    member.send_back(PlanReason::Missing);
    member.send_back(PlanReason::Missing);
    member.grant_another_send_back();
    let granted = member.plan().cloned().expect("the member keeps its plan");

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&granted),
        policy: &policy(),
    });

    assert_eq!(verdict, PlanVerdict::SendBack(PlanReason::Missing));
}

#[test]
fn the_plan_roles_the_file_names_are_the_ones_sent_back() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let keeping = keeping(0, PlanStage::Open);
    let policy = policy_of("[plan]\nroles = [\"coordinator\"]\n");

    let coordinator = decide_plan(PlanFacts {
        role: Role::Coordinator,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&keeping),
        policy: &policy,
    });
    let member = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&keeping),
        policy: &policy,
    });

    assert_eq!(coordinator, PlanVerdict::SendBack(PlanReason::Missing));
    assert_eq!(member, PlanVerdict::Accept);
}

#[test]
fn the_last_send_back_before_the_limit_still_goes_back() {
    let end = ended(RunOutcome::Done);
    let missing = PlanRead::Missing;
    let last = keeping(2, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Solo,
        end: &end,
        read: &missing,
        earlier: &[],
        keeping: Some(&last),
        policy: &policy(),
    });

    assert_eq!(verdict, PlanVerdict::SendBack(PlanReason::Missing));
}

#[test]
fn a_step_lost_by_a_version_already_sent_back_is_still_lost_when_the_same_done_comes_again() {
    let end = ended(RunOutcome::Done);
    let first = plan("Fix it — zażółć.\n\n1. Work\n[x] 1.1 Find it\n[ ] 1.2 Fix it\n");
    let lost = plan("Fix it — zażółć.\n\n1. Work\n[x] 1.1 Find it\n");
    let again = read("Fix it — zażółć.\n\n1. Work\n[x] 1.1 Find it\n");
    let sent_back = keeping(1, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Solo,
        end: &end,
        read: &again,
        earlier: &[first, lost],
        keeping: Some(&sent_back),
        policy: &policy(),
    });

    assert_eq!(
        verdict,
        PlanVerdict::SendBack(PlanReason::LostStep {
            steps: vec![item("1.2")]
        })
    );
}

#[test]
fn a_step_of_any_earlier_version_is_lost_once_each_in_the_order_it_first_appeared() {
    let end = ended(RunOutcome::Done);
    let first = plan("1. Work\n[x] 1.1 Find it\n[ ] 1.2 Fix it\n");
    let second = plan("1. Work\n[x] 1.1 Find it\n[ ] 1.2 Fix it\n[ ] 1.3 Test it\n");
    let third = plan("1. Work\n[x] 1.1 Find it\n[ ] 1.3 Test it\n");
    let now = read("1. Work\n[x] 1.1 Find it\n");
    let sent_back = keeping(1, PlanStage::Open);

    let verdict = decide_plan(PlanFacts {
        role: Role::Member,
        end: &end,
        read: &now,
        earlier: &[first, second, third],
        keeping: Some(&sent_back),
        policy: &policy(),
    });

    assert_eq!(
        verdict,
        PlanVerdict::SendBack(PlanReason::LostStep {
            steps: vec![item("1.2"), item("1.3")]
        })
    );
}

#[test]
fn a_coordinators_done_ends_a_turn_while_an_item_waits_or_is_with_a_member_and_the_work_otherwise() {
    use crate::state::{Completion, ItemPhase};

    let pending = ItemPhase::Pending;
    let with_member = ItemPhase::InProgress {
        assignee: "claude-1".parse().expect("an agent"),
        branch: "agents/claude-1/task-20261111-153001-k3x9-1.1"
            .parse()
            .expect("a branch"),
    };
    let done = ItemPhase::Done {
        completion: Completion::Listed,
    };
    let cases: &[(Role, &[&ItemPhase], DoneEnds)] = &[
        (Role::Coordinator, &[], DoneEnds::Work),
        (Role::Coordinator, &[&done], DoneEnds::Work),
        (Role::Coordinator, &[&done, &pending], DoneEnds::Turn),
        (Role::Coordinator, &[&with_member, &done], DoneEnds::Turn),
        (Role::Member, &[&pending, &with_member], DoneEnds::Work),
        (Role::Solo, &[&pending], DoneEnds::Work),
    ];

    for (role, phases, expected) in cases {
        assert_eq!(
            done_ends(*role, phases.iter().copied()),
            *expected,
            "{role:?} {phases:?}"
        );
    }
}
