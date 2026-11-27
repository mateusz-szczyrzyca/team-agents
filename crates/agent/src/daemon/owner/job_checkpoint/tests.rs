//! The jobs a start of the daemon finds going, read from the checkpoints they left.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::history::{CheckpointCause, CheckpointContent, CheckpointJob};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::kinds::Tier;
use agent_core::state::CommitId;
use agent_runtime::history::LastCheckpoint;
use agent_runtime::store::Home;

use super::{JobsOf, going_jobs};

/// A checkpoint of the task `task` of the job `job`, of `cause`, with the head `head` and the content `done` says.
fn checkpoint(task: &str, job: CheckpointJob, cause: CheckpointCause, head: &str, done: &str) -> LastCheckpoint {
    LastCheckpoint {
        at: agent_core::state::Timestamp::from_str("2026-11-27T10:00:00Z").unwrap(),
        task: task.parse().unwrap(),
        job,
        tier: Tier::Worker,
        pull_request: None,
        review_pass: None,
        cause,
        head: Some(CommitId::from_str(head).unwrap()),
        snapshot: Some(format!("refs/agents/snapshots/{task}/1")),
        content: Some(CheckpointContent {
            meant: "R1. Write the guide\nK1. exists docs/guide.md".to_owned(),
            done: done.to_owned(),
            left: "next: make the check pass and commit".to_owned(),
        }),
    }
}

/// The going jobs of the checkpoints `kept` of one round of a team whose repository is `example`.
fn going(kept: &[LastCheckpoint]) -> Vec<super::JobFacts> {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());
    let team: TeamId = "team-20261127-090000-aaaa".parse().unwrap();
    let round: RoundId = "round-20261127-090000-aaaa".parse().unwrap();
    let repo = "example".parse().unwrap();
    let whose = JobsOf {
        team: &team,
        round: &round,
        repo: &repo,
    };
    going_jobs(&home, &whose, kept)
        .into_iter()
        .map(|(facts, _)| facts)
        .collect()
}

const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";
const WORKER: &str = "task-20261127-090000-aaaa";

#[test]
fn a_worker_whose_check_failed_before_the_restart_goes_on_with_that_result() {
    let kept = [
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::RequestStarted,
            FIRST,
            "Commits: 0\nChanged files: none\nCheck: not run yet",
        ),
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::RequestChecked,
            SECOND,
            "Commits: 1\nChanged files: docs/a.md\nCheck: failed — guide missing",
        ),
    ];

    let jobs = going(&kept);

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].last_check, Some((false, "guide missing".to_owned())));
    assert_eq!(jobs[0].start.as_str(), FIRST);
}

#[test]
fn a_job_taken_over_starts_where_its_own_worktree_started_not_where_the_holder_before_left_off() {
    let kept = [
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::HolderChanged,
            FIRST,
            "Commits: 3\nCheck: not run yet",
        ),
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::RequestStarted,
            SECOND,
            "Commits: 0\nCheck: not run yet",
        ),
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::Commit,
            SECOND,
            "Commits: 1\nCheck: not run yet",
        ),
    ];

    let jobs = going(&kept);

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].start.as_str(), SECOND);
}

#[test]
fn a_job_whose_last_checkpoint_ended_it_is_not_going_and_a_job_handed_over_but_not_started_is_not_either() {
    let kept = [
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::RequestStarted,
            FIRST,
            "Check: not run yet",
        ),
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::RunEnded,
            SECOND,
            "Check: passed — ok",
        ),
        checkpoint(
            "task-20261127-090000-bbbb",
            CheckpointJob::Review,
            CheckpointCause::HolderChanged,
            FIRST,
            "Findings written so far: 0",
        ),
    ];

    assert!(going(&kept).is_empty());
}

#[test]
fn the_tasks_of_a_round_are_read_in_the_order_of_their_first_checkpoint() {
    let other = "task-20261127-090000-bbbb";
    let kept = [
        checkpoint(
            other,
            CheckpointJob::Investigation,
            CheckpointCause::InvestigationStarted,
            FIRST,
            "Commits: 0",
        ),
        checkpoint(
            WORKER,
            CheckpointJob::WorkerRequest,
            CheckpointCause::RequestStarted,
            FIRST,
            "Check: not run yet",
        ),
        checkpoint(
            other,
            CheckpointJob::Investigation,
            CheckpointCause::Commit,
            SECOND,
            "Commits: 1",
        ),
    ];

    let jobs = going(&kept);

    assert_eq!(
        jobs.iter().map(|facts| facts.task.clone()).collect::<Vec<TaskId>>(),
        vec![other.parse().unwrap(), WORKER.parse().unwrap()]
    );
}
