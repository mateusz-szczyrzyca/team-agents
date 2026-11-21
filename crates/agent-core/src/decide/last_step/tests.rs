use super::*;

use AfterLastStep::{AskHuman, Fail};

/// Every role, in the order of the table below.
const ROLES: [Role; 5] = [
    Role::Solo,
    Role::Coordinator,
    Role::Member,
    Role::Worker,
    Role::Reviewer,
];

#[test]
fn a_task_is_asked_after_its_last_step_only_when_it_was_taken_over_or_a_lead_holds_a_stopped_members_work() {
    use HeldBefore::{Nobody, Others};
    use MembersWork::{None, OfAStoppedMember};
    // (role, held before, members' work, what follows) for a task never asked, in modes auto and lead.
    let table = [
        (Role::Solo, Others, None, AskHuman),
        (Role::Solo, Nobody, None, Fail),
        (Role::Solo, Nobody, OfAStoppedMember, Fail),
        (Role::Member, Others, None, AskHuman),
        (Role::Member, Nobody, None, Fail),
        (Role::Member, Nobody, OfAStoppedMember, Fail),
        (Role::Coordinator, Others, None, AskHuman),
        (Role::Coordinator, Nobody, OfAStoppedMember, AskHuman),
        (Role::Coordinator, Nobody, None, Fail),
        (Role::Worker, Others, OfAStoppedMember, Fail),
        (Role::Reviewer, Others, OfAStoppedMember, Fail),
    ];
    for mode in [ReplacementMode::Auto, ReplacementMode::Lead] {
        for (role, held, work, expected) in table {
            assert_eq!(
                after_last_step(mode, role, Asked::Never, held, work),
                expected,
                "{mode:?} {role:?} {held:?} {work:?}"
            );
        }
    }
}

#[test]
fn a_task_asked_once_and_every_task_in_mode_off_fail_as_before() {
    for role in ROLES {
        for held in [HeldBefore::Nobody, HeldBefore::Others] {
            for work in [MembersWork::None, MembersWork::OfAStoppedMember] {
                assert_eq!(
                    after_last_step(ReplacementMode::Off, role, Asked::Never, held, work),
                    Fail,
                    "off {role:?} {held:?} {work:?}"
                );
                for mode in [ReplacementMode::Auto, ReplacementMode::Lead] {
                    assert_eq!(
                        after_last_step(mode, role, Asked::Once, held, work),
                        Fail,
                        "asked once {mode:?} {role:?} {held:?} {work:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn only_the_answer_a_starts_the_task_once_more_and_every_other_answer_stops_it() {
    let own = |text: &str| Verdict::Own(text.to_owned().try_into().unwrap());
    let table = [
        (Some(own("a")), LastStepAnswer::OnceMore),
        (Some(own("A")), LastStepAnswer::OnceMore),
        (Some(own("a) start it once more")), LastStepAnswer::OnceMore),
        (Some(own("a — ożywić")), LastStepAnswer::OnceMore),
        (Some(Verdict::Accept), LastStepAnswer::Stop),
        (Some(own("b")), LastStepAnswer::Stop),
        (Some(own("b) stop the task")), LastStepAnswer::Stop),
        (Some(own("again, please")), LastStepAnswer::Stop),
        (Some(own("ab")), LastStepAnswer::Stop),
        (None, LastStepAnswer::Stop),
    ];
    for (verdict, expected) in table {
        assert_eq!(last_step_answer(verdict.as_ref()), expected, "{verdict:?}");
    }
}
