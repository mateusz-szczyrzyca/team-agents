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
        files: Vec::new(),
        check: None,
        doing: None,
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
