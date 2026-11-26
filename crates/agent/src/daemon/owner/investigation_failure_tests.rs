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
