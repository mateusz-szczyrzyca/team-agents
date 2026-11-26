//! Team task guidance for requesting an investigation and receiving its answer.

#[test]
fn team_roles_explain_when_to_request_an_investigation_and_how_the_answer_arrives() {
    for (role, text) in [
        (
            "coordinator",
            include_str!("../../../prompts/harness/run/role-coordinator.md"),
        ),
        ("member", include_str!("../../../prompts/harness/run/role-member.md")),
    ] {
        for instruction in [
            "request_investigation",
            "Commit first",
            "answer arrives as a message",
            "only when you see a benefit",
        ] {
            assert!(text.contains(instruction), "{role} is missing {instruction}");
        }
    }
}
