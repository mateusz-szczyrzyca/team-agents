//! Diagnostic collection contracts at the runtime boundary.
use super::*;
use agent_core::config::AbsolutePath;
use agent_core::state::{Crew, Member, MergeMode, Participant, State, TaskListText, Team, Timestamp};
use std::os::unix::fs::PermissionsExt;

fn fixture() -> (tempfile::TempDir, Home, DebugArgs) {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);
    let task = "task-20261117-120000-a1b2".parse().unwrap();
    let team: TeamId = "team-20261117-120000-a1b2".parse().unwrap();
    let member = Member::new(task, Participant::Normal("codex-1".parse().unwrap()));
    let mut state = State::new();
    state
        .insert_team(Team::new(
            team.clone(),
            "example".parse().unwrap(),
            "2026-11-17T12:00:00Z".parse().unwrap(),
            MergeMode::Autopilot,
            TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Work\n".to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        ))
        .unwrap();
    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    std::fs::write(home.secrets_file(), "github_token = \"test-confidential-λ\"\n").unwrap();
    std::fs::set_permissions(home.secrets_file(), std::fs::Permissions::from_mode(0o600)).unwrap();
    (
        dir,
        home,
        DebugArgs {
            target: DebugTarget::Team(team),
            out: None,
            stale_minutes: None,
            daemon_log: None,
        },
    )
}

#[test]
fn collects_offline_team_without_mutating_state_and_redacts_raw_bytes() {
    let (_dir, home, args) = fixture();
    let task = "task-20261117-120000-a1b2".parse().unwrap();
    std::fs::create_dir_all(home.task_dir(&task)).unwrap();
    std::fs::write(home.task_dir(&task).join("prompt.md"), "test-confidential-λ prompt").unwrap();
    let before = std::fs::read(home.state_file()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(execute_debug(
        &home,
        args,
        Some(Err("daemon does not answer".to_owned())),
        &mut Vec::new(),
    ));
    assert!(matches!(result, Ok(DebugResult { .. })), "{result:?}");
    let result = result.unwrap();
    assert_eq!(std::fs::read(home.state_file()).unwrap(), before);
    assert_eq!(
        std::fs::read(result.bundle_dir.join("tasks").join(task.as_str()).join("prompt.md")).unwrap(),
        "<cut> prompt".as_bytes()
    );
    let summary = std::fs::read_to_string(result.summary_path).unwrap();
    assert!(summary.contains("daemon does not answer"), "{summary}");
    assert!(summary.contains("Not collected in this version."), "{summary}");
}

#[test]
fn retries_oversized_required_presentation_without_losing_full_facts() {
    use agent_core::debug_summary::{MachineSummary, SummaryInput, TeamSummaryFacts, UnreadablePartSummary};
    let input = SummaryInput {
        collected_at: "2026-11-17T12:00:00Z".parse::<Timestamp>().unwrap(),
        anomalies: vec![],
        daemon: None,
        team: TeamSummaryFacts {
            id: "team-20261117-120000-a1b2".parse().unwrap(),
            repo: "example".parse().unwrap(),
            items: vec![],
            rounds: vec![],
        },
        tasks: vec![],
        git: vec![],
        gate: vec![],
        history_tail: None,
        daemon_log_lines: None,
        bus_messages: vec![],
        notifications: vec![],
        machine: MachineSummary {
            load_avg: None,
            memory_total_bytes: None,
            memory_available_bytes: None,
            home_fs: None,
            temp_fs: None,
            daemon_descendant_pids: vec![],
            tasks_dir_bytes: None,
            worktrees_dir_bytes: None,
        },
        unreadable_parts: vec![UnreadablePartSummary {
            name: "worktree".to_owned(),
            reason: "λ".repeat(100_000),
        }],
    };
    assert!(matches!(
        agent_core::debug_summary::render_summary(&input, time::UtcOffset::UTC),
        Err(agent_core::debug_summary::SummaryError::RequiredFactsTooLarge { .. })
    ));
    let rendered = compact_summary(&input);
    assert!(matches!(rendered, Ok(_)), "{rendered:?}");
    let summary = rendered.unwrap();
    assert!(summary.len() < agent_core::debug_summary::MAX_SUMMARY_BYTES);
    assert!(summary.contains("worktree"));
    assert_eq!(input.unreadable_parts[0].reason.len(), 200_000);
}
