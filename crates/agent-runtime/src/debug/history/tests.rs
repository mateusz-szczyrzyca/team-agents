use std::path::PathBuf;

use crate::history::{HistoryDb, Store};
use agent_core::config::AbsolutePath;
use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunEndRecord, RunStart, RunWhy, Step, TeamStart};
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::MergeMode;

use super::*;

#[test]
fn record_run_id_extracts_run_ids() {
    let run: RunId = "run-20261118-060000-a1b2".parse().unwrap();
    let at = "2026-11-18T06:00:00Z".parse().unwrap();
    let team: TeamId = "team-20261118-060000-a1b2".parse().unwrap();
    let task: TaskId = "task-20261118-060000-a1b2".parse().unwrap();

    let team_record = Record::Team(Box::new(TeamStart {
        team: team.clone(),
        at,
        repo: "example".to_owned(),
        base_branch: "main".parse().unwrap(),
        merge: MergeMode::NoMerge,
        task_list: "Task list.".to_owned(),
        plan: Vec::new(),
    }));
    assert_eq!(record_run_id(&team_record), None);

    let run_start = Record::Run(Box::new(RunStart {
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
        prompt: "Prompt".to_owned(),
        worktree: AbsolutePath::try_from("/tmp/example".to_owned()).unwrap(),
        branch: "feature".parse().unwrap(),
        session: None,
    }));
    assert_eq!(record_run_id(&run_start), Some(&run));

    let run_end = Record::RunEnd(Box::new(RunEndRecord {
        run: run.clone(),
        at,
        end: agent_core::state::RunEnd::Stopped,
        final_message: Some("done".to_owned()),
    }));
    assert_eq!(record_run_id(&run_end), Some(&run));
}

#[test]
fn reads_live_history_from_active_wal_without_modifying_home_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    let history_path = temp_dir.path().join("history.db");
    let wal_path = temp_dir.path().join("history.db-wal");

    let team: TeamId = "team-20261118-060000-a1b2".parse().unwrap();
    let task: TaskId = "task-20261118-060000-a1b2".parse().unwrap();
    let run: RunId = "run-20261118-060000-a1b2".parse().unwrap();
    let at = "2026-11-18T06:00:00Z".parse().unwrap();

    let mut database = HistoryDb::open(&history_path, MaxTextBytes::default()).unwrap();
    database
        .write(&[
            Record::Team(Box::new(TeamStart {
                team: team.clone(),
                at,
                repo: "example".to_owned(),
                base_branch: "main".parse().unwrap(),
                merge: MergeMode::NoMerge,
                task_list: "Task list.".to_owned(),
                plan: Vec::new(),
            })),
            Record::Run(Box::new(RunStart {
                run: run.clone(),
                team: team.clone(),
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
                prompt: "Live prompt test".to_owned(),
                worktree: AbsolutePath::try_from("/tmp/worktree".to_owned()).unwrap(),
                branch: "feature".parse().unwrap(),
                session: None,
            })),
            Record::Step(Box::new(Step {
                run: run.clone(),
                number: std::num::NonZeroU32::MIN,
                at,
                event: agent_core::cli::ActivityEvent::Text("Live step".to_owned()),
            })),
            Record::RunEnd(Box::new(RunEndRecord {
                run,
                at,
                end: agent_core::state::RunEnd::Stopped,
                final_message: Some("Live finished".to_owned()),
            })),
        ])
        .unwrap();

    // Verify WAL file exists with contents from the active write
    assert!(wal_path.exists());
    let before_db_bytes = std::fs::read(&history_path).unwrap();
    let before_wal_bytes = std::fs::read(&wal_path).unwrap();

    let mut before_entries: Vec<PathBuf> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    before_entries.sort();

    let records = read_live_team_history(&history_path, &team).unwrap();

    // The live team history must collect all 4 records committed to the database/WAL
    assert_eq!(records.len(), 4);

    // The files in home must be byte-for-byte unchanged
    assert_eq!(std::fs::read(&history_path).unwrap(), before_db_bytes);
    assert_eq!(std::fs::read(&wal_path).unwrap(), before_wal_bytes);

    let mut after_entries: Vec<PathBuf> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    after_entries.sort();
    assert_eq!(after_entries, before_entries);
}

#[test]
fn rejects_oversized_history_database() {
    let temp_dir = tempfile::tempdir().unwrap();
    let history_path = temp_dir.path().join("history.db");
    let file = std::fs::File::create(&history_path).unwrap();
    file.set_len(MAX_HISTORY_BYTES.saturating_add(1)).unwrap();
    let team: TeamId = "team-20261118-060000-a1b2".parse().unwrap();

    let err = read_live_team_history(&history_path, &team).unwrap_err();
    assert!(
        matches!(
            err,
            HistoryCollectionError::TooLarge { ref path, size, limit }
            if *path == history_path && size == MAX_HISTORY_BYTES + 1 && limit == MAX_HISTORY_BYTES
        ),
        "{err:?}"
    );
}

#[test]
fn unreadable_history_metadata_is_not_reported_as_empty_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    std::os::unix::fs::symlink(&path, &path).unwrap();
    let team: TeamId = "team-20261118-060000-a1b2".parse().unwrap();
    let error = read_live_team_history(&path, &team).unwrap_err();
    assert!(matches!(error, HistoryCollectionError::Io { path: failed, .. } if failed == path));
}
