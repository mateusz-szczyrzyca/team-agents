//! Unit tests for publication preparation and error handling.

use std::sync::Arc;

use super::*;
use crate::daemon::owner::fixture::{Daemon, Sinks};
use crate::daemon::owner::rounds::Pulls;
use crate::daemon::owner::rounds::publish::PublishQuestionError;
use crate::daemon::pr::plan_publications;
use agent_core::status::TaskStatus;

fn n(value: u32) -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(value).unwrap()
}

async fn owner_of(state: &agent_core::state::State) -> (Daemon, Owner, Sinks, Pulls) {
    let daemon = Daemon::new(state, None);
    let secrets = daemon.home.root().join("secrets.toml");
    std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
    std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let (owner, sinks) = daemon.bare_owner().await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    let pulls = Pulls::new(
        Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        &owner.parts.settings,
    )
    .unwrap()
    .unwrap();
    (daemon, owner.with_pulls(pulls.clone()), sinks, pulls)
}

fn state_with_waiting_lead() -> (agent_core::state::State, agent_core::ids::TeamId) {
    let mut state = crate::fixtures::solo_state();
    let team_id = crate::fixtures::team_id(0);
    let item: agent_core::ids::ItemId = "1.1".parse().unwrap();
    let round: agent_core::ids::RoundId = "round-20261119-090000-wait".parse().unwrap();
    let branch: agent_core::ids::BranchName = "agents/claude-1/task-1-1.1".parse().unwrap();
    {
        let mut team = state.team_mut(&team_id).unwrap();
        team.open_round(round, Some(item), crate::fixtures::at(10)).unwrap();
        team.advance_item(
            item,
            agent_core::state::ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: branch.clone(),
            },
        )
        .unwrap();
        team.begin_run(
            &crate::fixtures::task(1),
            agent_core::state::NewRun {
                id: crate::fixtures::run(1),
                at: crate::fixtures::at(20),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
        let (_, mut lead) = team.find_mut(&crate::fixtures::task(1)).unwrap();
        lead.transition(TaskStatus::Running).unwrap();
        lead.end_run(
            crate::fixtures::at(30),
            agent_core::state::RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Done,
            },
        )
        .unwrap();
        lead.transition(TaskStatus::Waiting).unwrap();
    }
    (state, team_id)
}

#[test]
fn publish_question_error_display_omits_duplicate_source_formatting() {
    let task = crate::fixtures::task(1);
    let err_var = PublishQuestionError::Variables {
        task: task.clone(),
        source: agent_core::prompt::Variables::new()
            .with("dup", "1")
            .unwrap()
            .with("dup", "2")
            .unwrap_err(),
    };
    assert_eq!(
        err_var.to_string(),
        format!("cannot prepare publication variables for task {task}")
    );

    let err_render = PublishQuestionError::Render {
        task: task.clone(),
        source: agent_runtime::autopilot_prompts::RenderAutopilotTextError::NotLoaded {
            id: agent_core::prompt::AutopilotPrompt::NoticePublishQuestion,
        },
    };
    assert_eq!(
        err_render.to_string(),
        format!("cannot render publication question for task {task}")
    );

    let err_state = PublishQuestionError::State {
        task: task.clone(),
        source: agent_core::state::QuestionError::StillOpen { number: n(1) },
    };
    assert_eq!(
        err_state.to_string(),
        format!("cannot record publication question for task {task}")
    );
}

#[tokio::test]
async fn prepare_publish_question_file_propagates_unreadable_or_corrupt_file_error() {
    let (state, team_id) = state_with_waiting_lead();
    let (daemon, owner, _sinks, pulls) = owner_of(&state).await;
    let lead = owner.lead_of(&team_id).unwrap();
    let site = Site {
        pulls: &pulls,
        team: &team_id,
        lead: &lead,
    };
    let plan = plan_publications(owner.parts.state.team(&team_id).unwrap()).unwrap();
    let publication = &plan[0];

    let questions_file = daemon.home.questions_file(&lead.task, n(1));
    std::fs::create_dir_all(questions_file.parent().unwrap()).unwrap();
    // Write invalid UTF-8 bytes to simulate a corrupt questions file:
    std::fs::write(&questions_file, [0xff, 0xfe, 0xfd]).unwrap();

    let result = owner
        .prepare_publish_question_file(site, publication, &questions_file, "refused")
        .await;
    assert!(
        matches!(
            result,
            Err(PublishQuestionError::Read {
                source: crate::daemon::questions::QuestionsProblem::NotUtf8 { .. },
                ..
            })
        ),
        "must propagate corrupt questions file error instead of overwriting, got: {result:?}"
    );
    // File content must remain unchanged:
    assert_eq!(std::fs::read(&questions_file).unwrap(), [0xff, 0xfe, 0xfd]);
}

#[tokio::test]
async fn prepare_publish_question_file_atomic_no_clobber_reconciles_existing_file() {
    let (state, team_id) = state_with_waiting_lead();
    let (daemon, owner, _sinks, pulls) = owner_of(&state).await;
    let lead = owner.lead_of(&team_id).unwrap();
    let site = Site {
        pulls: &pulls,
        team: &team_id,
        lead: &lead,
    };
    let plan = plan_publications(owner.parts.state.team(&team_id).unwrap()).unwrap();
    let publication = &plan[0];

    let questions_file = daemon.home.questions_file(&lead.task, n(1));
    std::fs::create_dir_all(questions_file.parent().unwrap()).unwrap();
    let pre_existing_content = "# Questions\n\n## 1. Action?\n**Verdict:** a\n\n**ANSWERED**\n";
    std::fs::write(&questions_file, pre_existing_content).unwrap();

    let result = owner
        .prepare_publish_question_file(site, publication, &questions_file, "refused")
        .await;
    assert!(
        matches!(result, Ok(())),
        "must reconcile with existing valid file: {result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&questions_file).unwrap(),
        pre_existing_content,
        "existing questions file must not be overwritten"
    );
}

#[tokio::test]
async fn prepare_publish_question_file_returns_write_error_on_failure_and_recovers() {
    let (state, team_id) = state_with_waiting_lead();
    let (daemon, owner, _sinks, pulls) = owner_of(&state).await;
    let lead = owner.lead_of(&team_id).unwrap();
    let site = Site {
        pulls: &pulls,
        team: &team_id,
        lead: &lead,
    };
    let plan = plan_publications(owner.parts.state.team(&team_id).unwrap()).unwrap();
    let publication = &plan[0];

    // 1. Write failure when directory is missing / unwritable:
    let unwritable_file = daemon.home.root().join("nonexistent_parent_dir_1234/questions-1.md");
    let result = owner
        .prepare_publish_question_file(site, publication, &unwritable_file, "refused")
        .await;
    assert!(
        matches!(result, Err(PublishQuestionError::Write { .. })),
        "must return Write error on write failure, got: {result:?}"
    );

    // 2. Recoverability: once a valid writable path is used, preparation succeeds:
    let valid_file = daemon.home.questions_file(&lead.task, n(1));
    std::fs::create_dir_all(valid_file.parent().unwrap()).unwrap();
    let recovered = owner
        .prepare_publish_question_file(site, publication, &valid_file, "refused")
        .await;
    assert!(
        matches!(recovered, Ok(())),
        "pending escalation must succeed on valid path: {recovered:?}"
    );
    assert!(valid_file.exists());
    let content = std::fs::read_to_string(&valid_file).unwrap();
    assert!(content.contains("could not be published"));
}
