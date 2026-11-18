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
    let summary = rendered.unwrap();
    assert!(summary.len() < agent_core::debug_summary::MAX_SUMMARY_BYTES);
    assert!(summary.contains("worktree"));
    assert_eq!(input.unreadable_parts[0].reason.len(), 200_000);
}

#[test]
fn collects_runs_questions_gates_and_checks_and_names_unreadable_parts() {
    let (_dir, home, args) = fixture();
    let task: TaskId = "task-20261117-120000-a1b2".parse().unwrap();
    let run = "run-20261117-120001-a1b2".parse().unwrap();
    let run_dir = home.run_dir(&task, &run);
    std::fs::create_dir_all(&run_dir).unwrap();
    let files = [
        ("prompt.md", b"prompt".as_slice()),
        ("final.md", b"final"),
        ("stdout", &[0, 255, 1]),
        ("stderr", b"error"),
    ];
    for (name, bytes) in files {
        std::fs::write(run_dir.join(name), bytes).unwrap();
    }
    let question = "# Questions\n\n## 1. Proceed?\na) yes\n\n**Recommended:** a)\n\n**Verdict:**\n";
    std::fs::write(home.task_dir(&task).join("questions-2.md"), question).unwrap();
    std::fs::write(home.checks_file(&task), b"checks record").unwrap();
    std::fs::write(home.gate_record_file(&task), b"invalid gate record").unwrap();
    std::fs::write(home.gate_log(&task, std::num::NonZeroU32::MIN), b"gate output").unwrap();
    std::os::unix::fs::symlink(home.secrets_file(), home.task_dir(&task).join("secret-link")).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(execute_debug(&home, args, None, &mut Vec::new()));
    assert!(matches!(result, Ok(DebugResult { .. })), "{result:?}");
    let result = result.unwrap();
    let copied = result.bundle_dir.join("tasks").join(task.as_str());
    for (name, bytes) in files {
        assert_eq!(
            std::fs::read(copied.join("runs").join(run.as_str()).join(name)).unwrap(),
            bytes
        );
    }
    assert_eq!(
        std::fs::read(copied.join(home.checks_file(&task).file_name().unwrap())).unwrap(),
        b"checks record"
    );
    assert_eq!(std::fs::read(copied.join("gate-1.log")).unwrap(), b"gate output");
    assert_eq!(
        std::fs::read(copied.join("questions-2.md")).unwrap(),
        question.as_bytes()
    );
    assert!(!copied.join("secret-link").exists());
    let summary = std::fs::read_to_string(result.summary_path).unwrap();
    for name in ["questions-2.md", "secret-link", "history", "gate", "worktree"] {
        assert!(summary.contains(name), "missing {name}: {summary}");
    }
}

#[test]
fn summary_lists_commits_above_the_configured_base() {
    let (_dir, home, args) = fixture();
    let task: TaskId = "task-20261117-120000-a1b2".parse().unwrap();
    let repo = "example".parse().unwrap();
    let worktree = home.worktree_dir(&repo, &task);
    std::fs::create_dir_all(&worktree).unwrap();
    for arguments in [
        vec!["init", "--initial-branch=main"],
        vec!["commit", "--allow-empty", "-m", "Base"],
        vec!["switch", "-c", "feature"],
        vec!["commit", "--allow-empty", "-m", "Add café diagnostics"],
    ] {
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(&worktree)
            .args(["-c", "user.name=Example", "-c", "user.email=example@example.com"])
            .args(arguments)
            .env("GIT_AUTHOR_NAME", "Example")
            .env("GIT_AUTHOR_EMAIL", "example@example.com")
            .env("GIT_COMMITTER_NAME", "Example")
            .env("GIT_COMMITTER_EMAIL", "example@example.com")
            .env("GIT_AUTHOR_DATE", "2026-11-17T12:01:00Z")
            .env("GIT_COMMITTER_DATE", "2026-11-17T12:01:00Z")
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
    }
    std::fs::write(
        home.config_file(),
        "[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime
        .block_on(execute_debug(&home, args, None, &mut Vec::new()))
        .unwrap();
    let summary = std::fs::read_to_string(result.summary_path).unwrap();
    assert!(summary.contains("Add café diagnostics"), "{summary}");
    assert!(summary.contains("Example"), "{summary}");
}

#[test]
fn preserves_home_history_while_collecting_whole_prompts_steps_and_final_messages() {
    use crate::history::{HistoryDb, Store};
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunEndRecord, RunStart, RunWhy, Step};
    use agent_core::kinds::{CliKind, Role, Tier};
    let (_dir, home, args) = fixture();
    let run: agent_core::ids::RunId = "run-20261117-120001-a1b2".parse().unwrap();
    let at = "2026-11-17T12:00:00Z".parse().unwrap();
    let team = "team-20261117-120000-a1b2".parse().unwrap();
    let task = "task-20261117-120000-a1b2".parse().unwrap();
    let mut database = HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
    database
        .write(&[
            Record::Run(Box::new(RunStart {
                run: run.clone(),
                team,
                task,
                at,
                agent: AgentIdentity {
                    name: "codex-1".to_owned(),
                    tier: Tier::Normal,
                    cli: CliKind::Codex,
                    model: None,
                },
                role: Role::Solo,
                rounds: vec![],
                why: RunWhy::FirstRun,
                prompt: "Whole café prompt test-confidential-λ".to_owned(),
                worktree: AbsolutePath::try_from("/tmp/example".to_owned()).unwrap(),
                branch: "feature".parse().unwrap(),
                session: None,
            })),
            Record::Step(Box::new(Step {
                run: run.clone(),
                number: std::num::NonZeroU32::MIN,
                at,
                event: agent_core::cli::ActivityEvent::Text("Recorded café step".to_owned()),
            })),
            Record::RunEnd(Box::new(RunEndRecord {
                run,
                at,
                end: agent_core::state::RunEnd::Stopped,
                final_message: Some("Whole café final message".to_owned()),
            })),
        ])
        .unwrap();
    drop(database);
    let before = std::fs::read(home.history_file()).unwrap();
    let mut names_before: Vec<_> = std::fs::read_dir(home.root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names_before.sort();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime
        .block_on(execute_debug(&home, args, None, &mut Vec::new()))
        .unwrap();
    for file in ["history.txt", "history.jsonl"] {
        let text = std::fs::read_to_string(result.bundle_dir.join(file)).unwrap();
        for expected in ["Whole café prompt", "Recorded café step", "Whole café final message"] {
            assert!(text.contains(expected), "{file}: {text}");
        }
        assert!(!text.contains("test-confidential-λ"));
    }
    assert_eq!(std::fs::read(home.history_file()).unwrap(), before);
    let mut names_after: Vec<_> = std::fs::read_dir(home.root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "debug")
        .collect();
    names_after.sort();
    assert_eq!(names_after, names_before);
}

#[test]
fn reports_active_history_wal_without_ignoring_committed_records_or_changing_it() {
    let (_dir, home, args) = fixture();
    let database =
        crate::history::HistoryDb::open(&home.history_file(), agent_core::history::MaxTextBytes::default()).unwrap();
    let wal = home.root().join("history.db-wal");
    let before = std::fs::read(&wal).unwrap();
    assert_ne!(before, Vec::<u8>::new());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime
        .block_on(execute_debug(&home, args, None, &mut Vec::new()))
        .unwrap();
    let summary = std::fs::read_to_string(result.summary_path).unwrap();
    assert!(summary.contains("history has a nonempty WAL"), "{summary}");
    assert_eq!(std::fs::read(&wal).unwrap(), before);
    assert!(!result.bundle_dir.join("history.jsonl").exists());
    drop(database);
}

#[test]
fn resolve_target_known_team_returns_team() {
    let (_dir, home, args) = fixture();
    let state = State::from_json(&std::fs::read_to_string(home.state_file()).unwrap()).unwrap();
    let DebugTarget::Team(team_id) = args.target else {
        panic!("fixture targets a team");
    };
    assert_eq!(
        resolve_debug_target(&state, &DebugTarget::Team(team_id.clone())).unwrap(),
        team_id
    );
}

#[test]
fn resolve_target_unknown_team_returns_error() {
    let state = State::new();
    let id: TeamId = "team-20261117-230000-xxxx".parse().unwrap();
    let result = resolve_debug_target(&state, &DebugTarget::Team(id.clone()));
    assert!(matches!(result, Err(DebugRuntimeError::UnknownTarget { id: actual }) if actual == id.as_str()));
}

#[test]
fn resolve_target_unknown_task_returns_error() {
    let state = State::new();
    let id: TaskId = "task-20261117-230000-xxxx".parse().unwrap();
    let result = resolve_debug_target(&state, &DebugTarget::Task(id.clone()));
    assert!(matches!(result, Err(DebugRuntimeError::UnknownTarget { id: actual }) if actual == id.as_str()));
}

#[test]
fn resolve_target_unknown_round_returns_error() {
    let state = State::new();
    let id: RoundId = "round-20261117-230000-xxxx".parse().unwrap();
    let result = resolve_debug_target(&state, &DebugTarget::Round(id.clone()));
    assert!(matches!(result, Err(DebugRuntimeError::UnknownTarget { id: actual }) if actual == id.as_str()));
}

#[test]
fn redacts_secrets_before_compacting_required_summary_reasons() {
    let (_dir, home, args) = fixture();
    let reason = format!("{}test-confidential-λ{}", "z".repeat(110), "q".repeat(120_000));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime
        .block_on(execute_debug(&home, args, Some(Err(reason)), &mut Vec::new()))
        .unwrap();
    let summary = std::fs::read_to_string(result.summary_path).unwrap();
    assert!(!summary.contains("test-co"), "secret prefix survived compaction");
    assert!(summary.contains("<cut>"));
    assert!(summary.len() < agent_core::debug_summary::MAX_SUMMARY_BYTES);
}
