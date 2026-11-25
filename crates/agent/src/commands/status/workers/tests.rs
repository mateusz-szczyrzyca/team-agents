use std::num::NonZeroU64;
use std::str::FromStr;
use std::time::SystemTime;

use agent_core::config::AbsolutePath;
use agent_core::ids::AgentName;
use agent_runtime::agentbus::{DelegationStatus, HelperEntry, HelperVerdict, RequestId};
use time::UtcOffset;

use super::*;

/// The task of the team's member that asks for workers.
const MEMBER: &str = "task-20261111-153002-k3x9";

/// The worker's task of the requests.
const WORKER_TASK: &str = "task-20261125-075745-w8vj";

/// A request numbered `id` of the task `from_task`, in `status`, given to `worker`, for `step`, rated `verdict`.
fn record(
    id: u64,
    from_task: &str,
    status: DelegationStatus,
    worker: Option<&str>,
    step: Option<&str>,
    verdict: Option<HelperVerdict>,
) -> HelperRecord {
    HelperRecord {
        entry: HelperEntry {
            id: RequestId::new(NonZeroU64::new(id).unwrap()),
            from: "claude-1".parse().unwrap(),
            from_task: from_task.parse().unwrap(),
            repo: "team-agents".parse().unwrap(),
            branch: "agents/claude-1/test".parse().unwrap(),
            status,
            worker: worker.map(|name| name.parse::<AgentName>().unwrap()),
            requires: vec![],
            step: step.map(str::to_owned),
            request: "write the guide".to_owned(),
        },
        helper_task: worker.map(|_| WORKER_TASK.parse().unwrap()),
        verdict,
    }
}

/// The time `text` (RFC 3339).
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// A span from `start` to `end`.
fn span(start: Option<&str>, end: Option<&str>) -> Span {
    Span {
        start: start.map(at),
        end: end.map(at),
    }
}

/// The zone the lines of the tests are written in.
const UTC: Zone = Zone::Fixed(UtcOffset::UTC);

/// A done request of step 6.1 by `glm-1`, rated `verdict`.
fn done(verdict: Option<HelperVerdict>) -> HelperRecord {
    record(4, MEMBER, DelegationStatus::Done, Some("glm-1"), Some("6.1"), verdict)
}

/// The run of the sample worker: from 08:57 to 08:59 of one day.
fn same_day() -> Span {
    span(Some("2026-11-25T08:57:45Z"), Some("2026-11-25T08:59:30Z"))
}

// Rule: a worker whose result was used is accepted, with when it started and ended on one day.
#[test]
fn a_used_result_is_accepted_with_its_start_and_end() {
    assert_eq!(
        line(&done(Some(HelperVerdict::Used)), same_day(), UTC),
        "  worker H4  glm-1  step 6.1  2026-11-25 08:57–08:59 +00:00  accepted"
    );
}

#[test]
fn a_corrected_result_is_accepted_after_correction() {
    assert_eq!(
        line(&done(Some(HelperVerdict::Corrected)), same_day(), UTC),
        "  worker H4  glm-1  step 6.1  2026-11-25 08:57–08:59 +00:00  accepted after correction"
    );
}

#[test]
fn a_discarded_result_is_rejected() {
    assert_eq!(
        line(&done(Some(HelperVerdict::Discarded)), same_day(), UTC),
        "  worker H4  glm-1  step 6.1  2026-11-25 08:57–08:59 +00:00  rejected"
    );
}

#[test]
fn a_done_result_without_a_verdict_is_not_rated_yet() {
    assert_eq!(
        line(&done(None), same_day(), UTC),
        "  worker H4  glm-1  step 6.1  2026-11-25 08:57–08:59 +00:00  not rated yet"
    );
}

// Rule: a worker that works says since when.
#[test]
fn a_running_worker_says_since_when_it_works() {
    let running = record(4, MEMBER, DelegationStatus::Running, Some("glm-1"), Some("6.1"), None);

    assert_eq!(
        line(&running, span(Some("2026-11-25T08:57:45Z"), None), UTC),
        "  worker H4  glm-1  step 6.1  since 2026-11-25 08:57 +00:00  working"
    );
}

#[test]
fn a_starting_worker_without_a_step_has_not_started() {
    let starting = record(5, MEMBER, DelegationStatus::Starting, Some("glm-1"), None, None);

    assert_eq!(
        line(&starting, Span::default(), UTC),
        "  worker H5  glm-1  not started  working"
    );
}

#[test]
fn a_failed_worker_whose_end_was_not_written_says_so() {
    let failed = record(4, MEMBER, DelegationStatus::Failed, Some("glm-1"), Some("6.1"), None);

    assert_eq!(
        line(&failed, span(Some("2026-11-25T08:57:45Z"), None), UTC),
        "  worker H4  glm-1  step 6.1  2026-11-25 08:57 +00:00, end not recorded  failed"
    );
}

#[test]
fn an_unclear_request_asked_back() {
    let unclear = record(4, MEMBER, DelegationStatus::Unclear, Some("glm-1"), Some("6.1"), None);

    assert_eq!(
        line(&unclear, same_day(), UTC),
        "  worker H4  glm-1  step 6.1  2026-11-25 08:57–08:59 +00:00  unclear, asked back"
    );
}

#[test]
fn a_run_over_midnight_writes_both_dates() {
    assert_eq!(
        line(
            &done(Some(HelperVerdict::Used)),
            span(Some("2026-11-25T23:58:00Z"), Some("2026-11-26T00:03:00Z")),
            UTC
        ),
        "  worker H4  glm-1  step 6.1  2026-11-25 23:58 +00:00 – 2026-11-26 00:03 +00:00  accepted"
    );
}

// Rule: a team's workers are those that took a request of one of its members, by their numbers.
#[test]
fn a_team_has_the_requests_its_members_gave_to_workers_by_number() {
    let team = crate::commands::status::tests::team(2, false, vec![]);
    let member = team.members[1].task.as_str().to_owned();
    let records = vec![
        record(7, &member, DelegationStatus::Done, Some("glm-2"), Some("2.1"), None),
        record(3, &member, DelegationStatus::Running, Some("glm-1"), Some("1.1"), None),
        // Not taken by a worker yet.
        record(8, &member, DelegationStatus::Queued, None, Some("3.1"), None),
        // Another team's task.
        record(
            5,
            "task-20261124-114140-pq68",
            DelegationStatus::Running,
            Some("glm-3"),
            None,
            None,
        ),
    ];

    let numbers: Vec<u64> = of_team(&team, &records)
        .iter()
        .map(|record| record.entry.id.number().get())
        .collect();

    assert_eq!(numbers, [3, 7]);
}

/// A home in `dir` with the run `run` of the worker's task, whose outcome, when `ended` is given, was written then.
fn with_run(dir: &std::path::Path, run: &str, ended: Option<&str>) -> Home {
    let home = Home::new(&AbsolutePath::from_str(dir.to_str().unwrap()).unwrap());
    let run_dir = home.runs_dir(&WORKER_TASK.parse().unwrap()).join(run);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("out.jsonl"), "{}\n").unwrap();
    if let Some(ended) = ended {
        let outcome = run_dir.join("outcome.json");
        std::fs::write(&outcome, "{}").unwrap();
        let when = SystemTime::from(time::OffsetDateTime::from(at(ended).as_utc()));
        std::fs::File::options()
            .write(true)
            .open(&outcome)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }
    home
}

// Rule: the runs of a worker's task start at the ID of the first run and end when the last one wrote its outcome.
#[tokio::test]
async fn the_span_starts_with_the_first_run_and_ends_with_the_outcome_of_the_last() {
    let dir = tempfile::tempdir().unwrap();
    with_run(dir.path(), "run-20261125-075745-ec9d", Some("2026-11-25T08:01:00Z"));
    let home = with_run(dir.path(), "run-20261125-080500-ab12", Some("2026-11-25T08:59:30Z"));

    assert_eq!(
        span_of(&home, &WORKER_TASK.parse().unwrap()).await,
        span(Some("2026-11-25T07:57:45Z"), Some("2026-11-25T08:59:30Z"))
    );
}

#[tokio::test]
async fn a_last_run_that_goes_on_has_no_end() {
    let dir = tempfile::tempdir().unwrap();
    with_run(dir.path(), "run-20261125-075745-ec9d", Some("2026-11-25T08:01:00Z"));
    let home = with_run(dir.path(), "run-20261125-080500-ab12", None);

    assert_eq!(
        span_of(&home, &WORKER_TASK.parse().unwrap()).await,
        span(Some("2026-11-25T07:57:45Z"), None)
    );
}

#[tokio::test]
async fn an_entry_that_is_no_run_is_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let home = with_run(dir.path(), "run-20261125-075745-ec9d", Some("2026-11-25T08:01:00Z"));
    std::fs::write(home.runs_dir(&WORKER_TASK.parse().unwrap()).join("notes"), "x").unwrap();

    assert_eq!(
        span_of(&home, &WORKER_TASK.parse().unwrap()).await,
        span(Some("2026-11-25T07:57:45Z"), Some("2026-11-25T08:01:00Z"))
    );
}

#[tokio::test]
async fn a_task_without_runs_has_no_span() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());

    assert_eq!(span_of(&home, &WORKER_TASK.parse().unwrap()).await, Span::default());
}
