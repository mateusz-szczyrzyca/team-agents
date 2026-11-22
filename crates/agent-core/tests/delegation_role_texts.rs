//! The texts of the roles that keep a plan tell the agent when to give a step to a worker and when not to.

/// The lines on delegating that every role that keeps a plan is told.
const LINES: [&str; 5] = [
    "`delegate`",
    "`By: worker`",
    "one result, named files and a check a machine runs",
    "Never give a worker a decision, a design, a step whose check is `reviewer`",
    "do the step yourself",
];

#[test]
fn the_member_the_coordinator_and_the_solo_agent_are_told_when_to_delegate() {
    for (role, text) in [
        ("member", include_str!("../../../prompts/harness/run/role-member.md")),
        (
            "coordinator",
            include_str!("../../../prompts/harness/run/role-coordinator.md"),
        ),
        ("solo", include_str!("../../../prompts/harness/run/role-solo.md")),
    ] {
        for line in LINES {
            assert!(text.contains(line), "the {role}'s text does not say {line:?}");
        }
    }
}

#[test]
fn a_worker_is_not_told_to_delegate() {
    let text = include_str!("../../../prompts/harness/run/role-worker.md");

    assert!(!text.contains("`delegate`"), "{text}");
}
