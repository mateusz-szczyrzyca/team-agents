use agent_core::state::RunEnd;
use agent_core::status::{RunOutcome, TaskStatus};

use super::*;
use crate::daemon::team::support::{CHAIN, PLAIN, crew_of, ran, reported, solo, task_id};
use crate::daemon::team::{assignable, start_item};

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn told(team: &Team, n: u32, end: &RunEnd, message: Option<&str>) -> Option<Told> {
    let (role, member) = team.find(&task_id(n)).unwrap();
    notice_for(team, role, member, end, message).unwrap()
}

/// A team whose member (task 2) has the item 1.1.
fn working() -> Team {
    let mut team = crew_of(CHAIN, 1, 1);
    for given in assignable(&team).unwrap() {
        let mut progress = team.item_mut(given.item).unwrap();
        start_item(&mut progress, &given).unwrap();
    }
    team
}

#[test]
fn a_member_that_is_done_tells_the_coordinator_which_item_and_branch_are_ready() {
    let team = working();

    let told = told(&team, 2, &reported(RunOutcome::Done), Some("Finished.")).unwrap();

    assert_eq!(told.to, agent("inv-opus"));
    assert_eq!(
        told.notice,
        BusNotice::MemberFinished {
            member: agent("claude-1"),
            item: Some("1.1".parse().unwrap()),
            branch: "agents/claude-1/task-20261111-153002-k3x9-1.1".parse().unwrap(),
        }
    );
}

#[test]
fn a_member_of_a_list_without_items_that_is_done_names_no_item_and_its_task_branch() {
    let team = crew_of(PLAIN, 1, 0);

    let told = told(&team, 2, &reported(RunOutcome::Done), None).unwrap();

    assert_eq!(
        told.notice,
        BusNotice::MemberFinished {
            member: agent("claude-1"),
            item: None,
            branch: "agents/claude-1/task-20261111-153002-k3x9".parse().unwrap(),
        }
    );
}

#[test]
fn a_member_that_failed_says_so_with_the_end_of_its_message() {
    let team = working();

    let told = told(&team, 2, &reported(RunOutcome::Failed), Some("I could not do it.")).unwrap();

    assert_eq!(
        told.notice,
        BusNotice::MemberFailed {
            member: agent("claude-1"),
            message: "I could not do it.".to_owned(),
        }
    );
}

#[test]
fn a_member_that_ended_without_a_status_says_so() {
    let team = working();

    let told = told(&team, 2, &RunEnd::NoStatus { exit_code: Some(3) }, None).unwrap();

    assert_eq!(
        told.notice,
        BusNotice::MemberNoStatus {
            member: agent("claude-1"),
        }
    );
}

#[test]
fn the_ends_that_are_not_the_members_to_report_tell_nobody() {
    let team = working();
    for end in [
        reported(RunOutcome::Sleeping),
        reported(RunOutcome::Questions),
        reported(RunOutcome::Excluded),
        RunEnd::Stopped,
        RunEnd::Hung,
        RunEnd::RateLimited,
    ] {
        assert_eq!(told(&team, 2, &end, None), None, "{end:?}");
    }
}

#[test]
fn only_a_member_tells_the_coordinator() {
    let team = working();

    assert_eq!(
        told(&team, 1, &reported(RunOutcome::Done), None),
        None,
        "the coordinator"
    );
    assert_eq!(told(&team, 3, &reported(RunOutcome::Done), None), None, "a worker");
}

#[test]
fn a_solo_agent_has_nobody_to_tell() {
    let mut team = solo(CHAIN);
    ran(&mut team, 1, 1, reported(RunOutcome::Done), &[TaskStatus::Waiting]);

    assert_eq!(told(&team, 1, &reported(RunOutcome::Done), None), None);
}
