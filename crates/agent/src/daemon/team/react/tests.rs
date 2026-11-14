use agent_core::state::RunEnd;
use agent_core::status::{RunOutcome, TaskStatus};

use super::*;
use crate::daemon::team::assignable;
use crate::daemon::team::support::{CHAIN, PLAIN, crew_of, ran, reported, solo, start, task_id};

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
        start(&mut team, &given);
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

// ---- the end of a request for a worker ---------------------------------------------------------------------------

fn outcome(end: &RunEnd, status: TaskStatus, message: Option<&str>) -> Option<RequestOutcome> {
    request_outcome(end, status, message)
}

#[test]
fn a_worker_that_is_done_returns_its_report_without_the_status_line() {
    let found = outcome(
        &reported(RunOutcome::Done),
        TaskStatus::Waiting,
        Some("Result: three files.\nHow: ls src\nSTATUS: DONE"),
    );

    assert_eq!(
        found,
        Some(RequestOutcome::Done {
            summary: "Result: three files.\nHow: ls src".to_owned()
        })
    );
}

#[test]
fn a_report_that_is_only_the_status_line_is_returned_as_it_is_so_that_it_is_not_empty() {
    let found = outcome(&reported(RunOutcome::Done), TaskStatus::Waiting, Some("STATUS: DONE"));

    assert_eq!(
        found,
        Some(RequestOutcome::Done {
            summary: "STATUS: DONE".to_owned()
        })
    );
}

#[test]
fn a_worker_that_failed_returns_how_and_what_it_said() {
    let found = outcome(
        &reported(RunOutcome::Failed),
        TaskStatus::Failed,
        Some("I could not do it.\nSTATUS: FAILED"),
    );

    assert_eq!(
        found,
        Some(RequestOutcome::Failed {
            how: WorkerEnd::Failed,
            said: Some("I could not do it.\nSTATUS: FAILED".to_owned()),
        })
    );
}

#[test]
fn each_end_that_fails_a_request_says_how_the_worker_ended_and_nothing_it_did_not_say() {
    for (end, status, how) in [
        (reported(RunOutcome::Failed), TaskStatus::Failed, WorkerEnd::Failed),
        (reported(RunOutcome::Sleeping), TaskStatus::Waiting, WorkerEnd::NoResult),
        (reported(RunOutcome::Excluded), TaskStatus::Done, WorkerEnd::NoResult),
        (
            RunEnd::NoStatus { exit_code: None },
            TaskStatus::Failed,
            WorkerEnd::NoStatus,
        ),
        (RunEnd::Stopped, TaskStatus::Stopped, WorkerEnd::Stopped),
        (RunEnd::Hung, TaskStatus::Failed, WorkerEnd::Hung),
    ] {
        let found = outcome(&end, status, Some("  \n"));

        assert_eq!(found, Some(RequestOutcome::Failed { how, said: None }), "{end:?}");
    }
}

#[test]
fn the_ends_that_leave_no_result_fail_the_request() {
    for (end, status) in [
        (RunEnd::NoStatus { exit_code: Some(1) }, TaskStatus::Failed),
        (reported(RunOutcome::Sleeping), TaskStatus::Waiting),
        (reported(RunOutcome::Excluded), TaskStatus::Done),
        (RunEnd::Stopped, TaskStatus::Stopped),
        (RunEnd::Hung, TaskStatus::Failed),
    ] {
        let found = outcome(&end, status, None);

        assert!(
            matches!(found, Some(RequestOutcome::Failed { .. })),
            "{end:?}: {found:?}"
        );
    }
}

#[test]
fn a_worker_that_failed_after_its_run_says_how_by_the_end_of_that_run() {
    for (end, how, said) in [
        (
            reported(RunOutcome::Questions),
            WorkerEnd::NoResult,
            Some("I have to ask.\nSTATUS: QUESTIONS".to_owned()),
        ),
        (
            RunEnd::QuotaExhausted { resets_at: None },
            WorkerEnd::NoStatus,
            Some("I have to ask.\nSTATUS: QUESTIONS".to_owned()),
        ),
        (
            RunEnd::RateLimited,
            WorkerEnd::NoStatus,
            Some("I have to ask.\nSTATUS: QUESTIONS".to_owned()),
        ),
        (RunEnd::Hung, WorkerEnd::Hung, None),
    ] {
        let found = outcome(&end, TaskStatus::Failed, Some("I have to ask.\nSTATUS: QUESTIONS"));

        assert_eq!(found, Some(RequestOutcome::Failed { how, said }), "{end:?}");
    }
}

#[test]
fn a_request_goes_on_while_the_worker_is_paused_asks_or_is_queued_to_continue() {
    for (end, status) in [
        (RunEnd::QuotaExhausted { resets_at: None }, TaskStatus::Paused),
        (RunEnd::RateLimited, TaskStatus::Paused),
        (RunEnd::Hung, TaskStatus::Queued),
        (reported(RunOutcome::Questions), TaskStatus::Questions),
    ] {
        assert_eq!(outcome(&end, status, None), None, "{end:?}");
    }
}
