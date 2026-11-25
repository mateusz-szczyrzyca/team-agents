//! The lines of the requests for workers under a task in `agent status`.
use std::num::NonZeroU64;

use agent_runtime::agentbus::{DelegationStatus, RequestId, TaskRequest};

use super::*;

fn request(id: u64, step: Option<&str>, worker: Option<&str>, status: DelegationStatus) -> TaskRequest {
    TaskRequest {
        id: RequestId::new(NonZeroU64::new(id).unwrap()),
        step: step.map(|step| step.parse().unwrap()),
        worker: worker.map(|worker| worker.parse().unwrap()),
        status,
    }
}

#[test]
fn a_request_is_shown_with_its_step_its_worker_and_its_state() {
    let rows = [
        (
            request(3, Some("1.2"), Some("glm-1"), DelegationStatus::Done),
            "    request H3  step 1.2  worker glm-1  done",
        ),
        (
            request(4, Some("2.1"), None, DelegationStatus::Queued),
            "    request H4  step 2.1  no worker yet  queued",
        ),
        (
            request(5, None, Some("glm-2"), DelegationStatus::Running),
            "    request H5  no step  worker glm-2  running",
        ),
        (
            request(6, None, None, DelegationStatus::Failed),
            "    request H6  no step  no worker yet  failed",
        ),
    ];
    for (request, line) in rows {
        assert_eq!(request_line(&request), line);
    }
}

#[tokio::test]
async fn requests_that_cannot_be_read_say_why_and_unread_ones_say_nothing() {
    let task: TaskId = "task-20261122-084934-hogq".parse().unwrap();

    let unknown = told(&Source::Unknown("cannot read the settings".to_owned()), [&task]).await;
    let unread = told(&Source::Unread, [&task]).await;

    assert_eq!(unknown.of(&task), ["    requests unknown (cannot read the settings)"]);
    assert_eq!(unread.of(&task), [] as [String; 0]);
}

#[tokio::test]
async fn a_bus_that_cannot_be_run_says_why_under_each_task() {
    let task: TaskId = "task-20261122-084934-hogq".parse().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-agentbus");
    let bus = AgentBus::new(
        std::str::FromStr::from_str(missing.to_str().unwrap()).unwrap(),
        std::str::FromStr::from_str(dir.path().to_str().unwrap()).unwrap(),
    );

    let told = told(&Source::Bus(bus), [&task]).await;

    let [line] = told.of(&task) else {
        panic!("{:?}", told.of(&task))
    };
    assert!(
        line.starts_with("    requests unknown (cannot run agentbus helper-dump"),
        "{line}"
    );
}

fn shown(now: &str) -> Shown {
    Shown {
        now: now.parse().unwrap(),
        zone: Zone::Fixed(time::UtcOffset::from_hms(1, 0, 0).unwrap()),
    }
}

#[test]
fn a_queued_request_says_since_when_it_waits_in_the_zone_of_the_status() {
    let queued = request(3, Some("2.1"), None, DelegationStatus::Queued);

    let line = request_line_waiting(
        &queued,
        Some("2026-11-25T20:10:30Z".parse().unwrap()),
        shown("2026-11-25T20:22:45Z"),
    );

    assert_eq!(
        line,
        "    request H3  step 2.1  no worker yet  queued  waiting since 21:10 (12 min)"
    );
}

#[test]
fn the_minutes_of_a_wait_are_whole_and_never_negative() {
    let queued = request(4, None, None, DelegationStatus::Queued);
    let since = Some("2026-11-25T20:10:00Z".parse().unwrap());

    assert_eq!(
        request_line_waiting(&queued, since, shown("2026-11-25T20:10:59Z")),
        "    request H4  no step  no worker yet  queued  waiting since 21:10 (0 min)"
    );
    assert_eq!(
        request_line_waiting(&queued, since, shown("2026-11-25T20:00:00Z")),
        "    request H4  no step  no worker yet  queued  waiting since 21:10 (0 min)"
    );
}

#[test]
fn a_request_without_a_note_or_not_queued_says_nothing_of_a_wait() {
    let rows = [
        (request(5, None, None, DelegationStatus::Queued), None),
        (
            request(6, None, Some("glm-1"), DelegationStatus::Running),
            Some("2026-11-25T20:10:00Z".parse().unwrap()),
        ),
    ];
    for (request, since) in rows {
        assert_eq!(
            request_line_waiting(&request, since, shown("2026-11-25T20:30:00Z")),
            request_line(&request)
        );
    }
}

#[tokio::test]
async fn the_note_of_a_queued_request_in_its_task_directory_gives_the_wait() {
    let dir = tempfile::tempdir().unwrap();
    let queued = request(3, Some("2.1"), None, DelegationStatus::Queued);
    worker_wait::write_since(dir.path(), queued.id, "2026-11-25T20:10:00Z".parse().unwrap())
        .await
        .unwrap();

    let line = line_of(&queued, dir.path(), shown("2026-11-25T20:22:00Z")).await;

    assert_eq!(
        line,
        "    request H3  step 2.1  no worker yet  queued  waiting since 21:10 (12 min)"
    );
}

#[tokio::test]
async fn a_queued_request_without_a_note_is_shown_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let queued = request(3, Some("2.1"), None, DelegationStatus::Queued);

    let line = line_of(&queued, dir.path(), shown("2026-11-25T20:22:00Z")).await;

    assert_eq!(line, "    request H3  step 2.1  no worker yet  queued");
}

#[tokio::test]
async fn a_note_that_cannot_be_read_is_said_beside_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let queued = request(3, Some("2.1"), None, DelegationStatus::Queued);
    std::fs::write(worker_wait::wait_path(dir.path(), queued.id), "damaged").unwrap();

    let line = line_of(&queued, dir.path(), shown("2026-11-25T20:22:00Z")).await;

    assert!(
        line.starts_with("    request H3  step 2.1  no worker yet  queued  waiting since unknown ("),
        "{line}"
    );
}
