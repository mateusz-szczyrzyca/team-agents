//! An agent its provider refused is given no new item while it is refused: the coordinator or the next fitting member
//! takes it, and an agent that is not refused is unaffected.

use super::*;
use crate::decide::replacement::ReplacementMode;
use crate::kinds::Tier;
use crate::state::fixtures::{at, inv, member, normal, team_id};
use crate::state::{Crew, MergeMode, TaskListText, TeamMembers};

/// The agent named `name`.
fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

/// A team whose coordinator `lead` is an investigator and whose one member `member` is a normal agent, with one item of
/// tier `normal`.
fn team() -> Team {
    Team::new(
        team_id(1),
        "example".parse().unwrap(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: normal\n".to_owned()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, inv("lead")),
            members: vec![member(2, normal("member"))],
            workers: vec![],
        })
        .unwrap(),
    )
}

#[test]
fn a_refused_member_gets_no_item_and_the_item_waits_as_for_a_busy_one() {
    let item = "1.1".parse().unwrap();
    let refused = [agent("member")];

    let chosen = dispatch_refusing(&team(), ReplacementMode::Auto, &[], &[], &refused);

    assert_eq!(
        chosen,
        Dispatch {
            assigned: vec![],
            waiting: vec![Waiting {
                item,
                required: Tier::Normal,
                reason: WaitReason::Busy,
            }],
        }
    );
}

#[test]
fn an_agent_that_is_not_refused_keeps_its_item() {
    let item = "1.1".parse().unwrap();
    let refused = [agent("someone-else")];

    let chosen = dispatch_refusing(&team(), ReplacementMode::Auto, &[], &[], &refused);

    assert_eq!(
        chosen,
        Dispatch {
            assigned: vec![(item, normal("member"))],
            waiting: vec![],
        }
    );
}
