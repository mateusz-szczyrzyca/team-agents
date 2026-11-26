//! Failed investigation attempts retain private text and retry a different investigator.

use std::os::unix::fs::PermissionsExt;

use agent_core::kinds::Role;
use agent_core::state::{InvestigationRecord, Member, RunEnd};
use agent_core::status::RunOutcome;

use super::fixture::{Daemon, pair_state};
use crate::fixtures::{at, investigator, task, team_id};

async fn first_failure(end: RunEnd) {
    let dir = tempfile::tempdir().unwrap();
    let bus = dir.path().join("bus");
    std::fs::write(&bus, "#!/bin/sh\nset -eu\nroot=$(dirname \"$0\")\ncase \"$1\" in\ninv-list) printf '#1 running\\n';;\ninv-get) cat \"$root/request.json\";;\n*) printf '%s\\n' \"$@\" >> \"$root/calls\";;\nesac\n").unwrap();
    std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = dir.path().join("answer.md");
    std::fs::write(&file, "# Investigation\n").unwrap();
    let request = serde_json::json!({"id":1,"from_agent":"claude-2","from_task":task(2),"branch":"main","question":"help","status":"running","inv_task":task(3),"file":file});
    std::fs::write(dir.path().join("request.json"), serde_json::to_vec(&request).unwrap()).unwrap();
    let mut state = pair_state();
    state
        .add_reviewer(
            &team_id(0),
            Member::new(task(3), investigator("inv-1")).with_role(Role::Investigator),
        )
        .unwrap();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .record_investigation(InvestigationRecord {
            id: std::num::NonZeroU32::MIN,
            task: task(3),
            started: at(0),
            failed_with: vec![],
        });
    let daemon = Daemon::new(&state, Some(&bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(3)).unwrap();
    owner
        .investigation_ended(&task(3), &located, &end, Some("Private unfinished résumé"))
        .await
        .unwrap();
    let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap_or_default();
    assert!(calls.contains("queued"), "{calls}");
    assert!(!calls.contains("send"), "{calls}");
    let text = std::fs::read_to_string(file).unwrap();
    assert!(text.contains("## Unfinished answer of inv-1 ("), "{text}");
    assert!(text.contains("Private unfinished résumé"), "{text}");
    assert_eq!(
        owner.parts.state.team(&team_id(0)).unwrap().investigations()[0].failed_with,
        vec!["inv-1".parse().unwrap()]
    );
}

#[tokio::test]
async fn first_failed_status_requeues_the_request() {
    first_failure(RunEnd::Reported {
        outcome: RunOutcome::Failed,
    })
    .await;
}
#[tokio::test]
async fn first_cut_off_answer_is_private_and_requeued() {
    first_failure(RunEnd::NoStatus { exit_code: Some(0) }).await;
}
#[tokio::test]
async fn first_hung_investigator_requeues_the_request() {
    first_failure(RunEnd::Hung).await;
}
#[tokio::test]
async fn first_turn_limit_requeues_the_request() {
    first_failure(RunEnd::TurnLimit).await;
}
#[tokio::test]
async fn first_rate_limit_requeues_the_request() {
    first_failure(RunEnd::RateLimited).await;
}
#[tokio::test]
async fn first_quota_refusal_requeues_the_request() {
    first_failure(RunEnd::QuotaExhausted { resets_at: None }).await;
}

/// An isolated bus boundary whose calls can be inspected without a running daemon.
struct Attempt {
    dir: tempfile::TempDir,
    bus: std::path::PathBuf,
    state: agent_core::state::State,
}

impl Attempt {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bus = dir.path().join("bus");
        std::fs::write(&bus, "#!/bin/sh\nset -eu\nroot=$(dirname \"$0\")\ncase \"$1\" in\ninv-list) printf '#1 open\\n';;\nsend) printf '%s\\n' \"$@\" >> \"$root/calls\"; printf '[#1 sent]\\n';;\ninv-get) cat \"$root/request.json\";;\n*) printf '%s\\n' \"$@\" >> \"$root/calls\";;\nesac\n").unwrap();
        std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut state = pair_state();
        state
            .add_reviewer(
                &team_id(0),
                Member::new(task(3), investigator("inv-1")).with_role(Role::Investigator),
            )
            .unwrap();
        state
            .team_mut(&team_id(0))
            .unwrap()
            .record_investigation(InvestigationRecord {
                id: std::num::NonZeroU32::MIN,
                task: task(3),
                started: at(0),
                failed_with: vec![],
            });
        let attempt = Self { dir, bus, state };
        attempt.request("running", Some(task(3)));
        attempt
    }

    fn request(&self, status: &str, assigned: Option<agent_core::ids::TaskId>) {
        let request = serde_json::json!({"id":1,"from_agent":"claude-2","from_task":task(2),"branch":"main","question":"help","status":status,"inv_task":assigned,"file":self.dir.path().join("answer.md")});
        std::fs::write(
            self.dir.path().join("request.json"),
            serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
    }

    fn calls(&self) -> String {
        match std::fs::read_to_string(self.dir.path().join("calls")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("bus calls: {error}"),
        }
    }
}

#[tokio::test]
async fn the_same_request_failing_a_second_time_notifies_the_requester() {
    let mut attempt = Attempt::new();
    attempt
        .state
        .team_mut(&team_id(0))
        .unwrap()
        .record_investigation(InvestigationRecord {
            id: std::num::NonZeroU32::MIN,
            task: task(3),
            started: at(0),
            failed_with: vec!["inv-other".parse().unwrap()],
        });
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(3)).unwrap();
    owner
        .investigation_ended(&task(3), &located, &RunEnd::Hung, Some("private"))
        .await
        .unwrap();
    let calls = attempt.calls();
    assert!(calls.contains("failed"), "{calls}");
    assert!(calls.contains("send"), "{calls}");
    assert!(calls.contains("decide yourself"), "{calls}");
    assert!(calls.contains("questions file"), "{calls}");
    assert!(!calls.contains("private"), "{calls}");
}

#[tokio::test]
async fn a_human_stop_fails_without_a_retry() {
    let attempt = Attempt::new();
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(3)).unwrap();
    owner
        .investigation_ended(&task(3), &located, &RunEnd::Stopped, None)
        .await
        .unwrap();
    assert!(attempt.calls().contains("stopped by the human"));
    assert!(!attempt.calls().contains("queued"));
}

#[tokio::test]
async fn an_ended_requester_fails_without_a_message() {
    let mut attempt = Attempt::new();
    attempt.state.remove_team(&team_id(0)).unwrap();
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.reconcile_investigations().await.unwrap();
    assert!(attempt.calls().contains("the requester's task ended"));
    assert!(!attempt.calls().contains("send"));
}

#[tokio::test]
async fn a_restart_requeues_an_assignment_without_a_live_task() {
    let attempt = Attempt::new();
    attempt.request("starting", Some(task(9)));
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.reconcile_investigations().await.unwrap();
    assert!(attempt.calls().contains("queued"));
}

#[tokio::test]
async fn a_restart_preserves_an_assignment_with_a_live_task() {
    let attempt = Attempt::new();
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.reconcile_investigations().await.unwrap();
    assert_eq!(attempt.calls(), "");
}

#[tokio::test]
async fn an_answer_timeout_stops_the_run_and_fails_without_retry() {
    let attempt = Attempt::new();
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let cancel = tokio_util::sync::CancellationToken::new();
    owner.cancels.insert(task(3), cancel.clone());
    daemon.clock.advance(time::Duration::minutes(30));
    owner.reconcile_investigations().await.unwrap();
    assert!(attempt.calls().contains("no answer in 30 minutes"));
    assert!(cancel.is_cancelled());
    assert!(!attempt.calls().contains("queued"));
}

#[tokio::test]
async fn time_in_the_queue_does_not_count_toward_answer_minutes() {
    let attempt = Attempt::new();
    attempt.request("queued", None);
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    daemon.clock.advance(time::Duration::minutes(40));
    owner.reconcile_investigations().await.unwrap();
    assert_eq!(attempt.calls(), "");
    owner
        .parts
        .state
        .team_mut(&team_id(0))
        .unwrap()
        .record_investigation(InvestigationRecord {
            id: std::num::NonZeroU32::MIN,
            task: task(3),
            started: agent_core::state::Timestamp::now(&owner.parts.clock).unwrap(),
            failed_with: vec![],
        });
    attempt.request("running", Some(task(3)));
    owner.reconcile_investigations().await.unwrap();
    assert_eq!(attempt.calls(), "");
}

#[tokio::test]
async fn an_investigators_quota_refusal_does_not_pause_its_task_or_change_its_agent() {
    let attempt = Attempt::new();
    let daemon = Daemon::new(&attempt.state, Some(&attempt.bus));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(3)).unwrap();
    let end = RunEnd::QuotaExhausted { resets_at: None };
    let settled = crate::daemon::settle::settle(Role::Investigator, &end, None, None);
    let (status, notice) = owner
        .settled_status(
            super::Ended {
                task: &task(3),
                located: &located,
                now: at(0),
                settled,
                progress: agent_runtime::run::Progress::Nothing,
                end,
                used_tool: false,
            },
            &mut vec![],
        )
        .await
        .unwrap();
    assert_eq!(status, agent_core::status::TaskStatus::Failed);
    assert!(notice.is_none());
    assert_eq!(owner.locate(&task(3)).unwrap().agent, "inv-1");
}
