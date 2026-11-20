//! Golden protocol for reporting and recovering delivered items.
use agent_core::prompt::{TeamPrompt, Variables};

#[test]
fn delivery_roles_match_the_protocol_goldens() {
    for (actual, expected) in [
        (
            include_str!("../../../prompts/harness/run/role-coordinator.md"),
            include_str!("fixtures/delivery-role-coordinator.md"),
        ),
        (
            include_str!("../../../prompts/harness/run/role-member.md"),
            include_str!("fixtures/delivery-role-member.md"),
        ),
    ] {
        assert_eq!(actual, expected);
    }
}

#[test]
fn delivery_close_message_explains_the_reason_and_the_next_step() {
    let values = Variables::new().with("item", "2.1").unwrap().with("pr", "7").unwrap();
    let rendered = TeamPrompt::DeliveryClosed
        .spec()
        .render(
            include_str!("../../../prompts/harness/team/bus-delivery-closed.md"),
            &values,
        )
        .unwrap();
    assert_eq!(
        rendered,
        "The delivery claim for item 2.1 ended: PR #7 was closed without merging. The member resumes the item in a fresh round on its branch. Integrate its work and publish the item's own pull request normally. Reopening PR #7 does not restore this claim.\n"
    );
}
