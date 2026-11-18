//! Regression contracts for collected item and worktree facts.
use super::*;
use agent_core::state::{Crew, MergeMode, Participant, TaskListText};

fn team() -> Team {
    Team::new(
        "team-20261117-120000-a1b2".parse().unwrap(),
        "example".parse().unwrap(),
        "2026-11-17T12:00:00Z".parse().unwrap(),
        MergeMode::Autopilot,
        TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Diagnose café\n".to_owned()).unwrap(),
        Crew::solo(Member::new(
            "task-20261117-120000-a1b2".parse().unwrap(),
            Participant::Normal("codex-1".parse().unwrap()),
        ))
        .unwrap(),
    )
}

#[test]
fn item_summary_uses_the_task_list_title() {
    let team = team();
    let collection = Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    assert_eq!(collection.summary.team.items[0].title, "Diagnose café");
}

fn git(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["-c", "user.name=Example", "-c", "user.email=example@example.com"])
        .args(args)
        .env("GIT_AUTHOR_NAME", "Example")
        .env("GIT_AUTHOR_EMAIL", "example@example.com")
        .env("GIT_COMMITTER_NAME", "Example")
        .env("GIT_COMMITTER_EMAIL", "example@example.com")
        .env("GIT_AUTHOR_DATE", "2026-11-17T12:01:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-17T12:01:00Z")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn git_collection_keeps_tracked_paths_branch_refs_and_completed_details() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = team();
    let task = team.crew().roster()[0].1.task().clone();
    let worktree = home.worktree_dir(team.repo(), &task);
    std::fs::create_dir_all(&worktree).unwrap();
    git(&worktree, &["init", "--initial-branch=main"]);
    std::fs::write(worktree.join("old.txt"), "tracked\n").unwrap();
    git(&worktree, &["add", "old.txt"]);
    git(&worktree, &["commit", "-m", "Base"]);
    git(&worktree, &["switch", "-c", "feature/café"]);
    git(&worktree, &["commit", "--allow-empty", "-m", "Collect café"]);
    for name in [
        "refs/team-agents/published/feature/café",
        "refs/team-agents/reviewed/feature/café",
        "refs/team-agents/published/unrelated",
        "refs/team-agents/reviewed/unrelated",
        "refs/team-agents/other/feature/café",
    ] {
        git(&worktree, &["update-ref", name, "HEAD"]);
    }
    git(&worktree, &["mv", "old.txt", "new café\nname.txt"]);
    std::fs::write(worktree.join("untracked.txt"), "untracked\n").unwrap();
    std::fs::write(
        home.config_file(),
        "[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    runtime.block_on(collection.observe(&home, &team, None)).unwrap();
    let facts = &collection.summary.git[0];
    assert_eq!(facts.uncommitted_files, vec![PathBuf::from("new café\nname.txt")]);
    assert_eq!(
        facts.harness_refs,
        vec![
            format!("refs/team-agents/published/feature/café {}", facts.tip),
            format!("refs/team-agents/reviewed/feature/café {}", facts.tip),
        ]
    );
    assert_eq!(facts.lead_contains_tip, Some(true));
    assert_eq!(
        facts
            .commits_over_base
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["Collect café"]
    );
    let artifact = collection
        .artifacts
        .iter()
        .find(|(path, _)| path == &PathBuf::from("git").join(format!("{task}.txt")))
        .unwrap();
    assert_eq!(artifact.1, format!("{facts:#?}").as_bytes());
    assert_eq!(
        collection.assessment.lead_git.as_ref().map(|lead| lead.dirty),
        Some(true)
    );
}

#[test]
fn untracked_files_make_lead_dirty_without_becoming_tracked_changes() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = team();
    let task = team.crew().roster()[0].1.task().clone();
    let worktree = home.worktree_dir(team.repo(), &task);
    std::fs::create_dir_all(&worktree).unwrap();
    git(&worktree, &["init", "--initial-branch=main"]);
    git(&worktree, &["commit", "--allow-empty", "-m", "Base"]);
    std::fs::write(worktree.join("untracked.txt"), "untracked\n").unwrap();
    std::fs::write(
        home.config_file(),
        "[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    runtime.block_on(collection.observe(&home, &team, None)).unwrap();
    assert_eq!(collection.summary.git[0].uncommitted_files, Vec::<PathBuf>::new());
    assert_eq!(
        collection.assessment.lead_git.as_ref().map(|lead| lead.dirty),
        Some(true)
    );
}

#[test]
fn rejects_malformed_porcelain_records_including_missing_rename_sources() {
    for bytes in [b"invalid\0".as_slice(), b"R  destination\0".as_slice()] {
        assert!(matches!(parse_status(bytes), Err(ObservationError::StatusFields)));
    }
}

#[test]
fn porcelain_paths_are_lossless_and_rename_sources_are_not_changes() {
    let cases = [
        (b"".as_slice(), Vec::<PathBuf>::new()),
        (b" M ordinary.txt\0".as_slice(), vec![PathBuf::from("ordinary.txt")]),
        (
            b"R  destination\0source\0?? ignored\0".as_slice(),
            vec![PathBuf::from("destination")],
        ),
        (b" C copy\0source\0".as_slice(), vec![PathBuf::from("copy")]),
        (
            b"A  \xff\n.txt\0".as_slice(),
            vec![PathBuf::from(OsString::from_vec(b"\xff\n.txt".to_vec()))],
        ),
    ];
    for (bytes, expected) in cases {
        assert_eq!(parse_status(bytes).unwrap(), expected);
    }
}

#[test]
fn git_failure_presentation_escapes_bytes_as_readable_text() {
    let error = ObservationError::Git {
        exit: ExitReport::Exited { code: 128 },
        stderr: b"fatal: missing \xff\n".to_vec(),
    };
    assert_eq!(
        error.to_string(),
        r#"read-only git command exited Exited { code: 128 }: "fatal: missing \xFF\n""#
    );
}

#[test]
fn last_run_endings_are_diagnostic_prose() {
    use agent_core::state::RunEnd;
    use agent_core::status::RunOutcome;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&dir.path().to_str().unwrap().parse().unwrap());
    let team = team();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cases = [
        (
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
            "reported DONE",
        ),
        (
            RunEnd::NoStatus { exit_code: Some(128) },
            "without a status, exited with 128",
        ),
        (
            RunEnd::NoStatus { exit_code: None },
            "without a status, terminated by signal",
        ),
        (RunEnd::QuotaExhausted { resets_at: None }, "quota exhausted"),
        (
            RunEnd::QuotaExhausted {
                resets_at: Some("2026-11-18T12:00:00Z".parse().unwrap()),
            },
            "resets at 2026-11-18",
        ),
        (RunEnd::RateLimited, "rate limited"),
        (RunEnd::Hung, "stopped by the watchdog"),
        (RunEnd::TurnLimit, "turn limit reached"),
        (RunEnd::Stopped, "stopped by the human"),
    ];
    for (end, expected) in cases {
        let mut member = team.crew().roster()[0].1.clone();
        member
            .begin_run(
                "run-20261117-120001-a1b2".parse().unwrap(),
                "2026-11-17T12:00:01Z".parse().unwrap(),
                None,
            )
            .unwrap();
        member.end_run("2026-11-17T12:00:02Z".parse().unwrap(), end).unwrap();
        std::fs::create_dir_all(home.task_dir(member.task())).unwrap();
        let mut collection =
            Collection::new(&team, "2026-11-17T12:01:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
        runtime.block_on(collection.task(&home, &member, Role::Solo)).unwrap();
        let detail = collection.summary.tasks[0].last_run_end_detail.as_ref().unwrap();
        assert!(detail.contains(expected), "expected {expected}: {detail}");
        assert!(detail.contains("ended"));
        assert!(!detail.contains("Timestamp("));
    }
}

#[test]
fn questions_are_numeric_and_gate_tails_name_source_and_time() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&dir.path().to_str().unwrap().parse().unwrap());
    let team = team();
    let member = team.crew().roster()[0].1;
    let task_dir = home.task_dir(member.task());
    std::fs::create_dir_all(&task_dir).unwrap();
    let question = "# Questions\n\n## 1. Proceed?\na) yes\n\n**Recommended:** a)\n\n**Verdict:**\n";
    for n in [10, 1, 2] {
        std::fs::write(task_dir.join(format!("questions-{n}.md")), question).unwrap();
        std::fs::write(task_dir.join(format!("gate-{n}.log")), format!("output {n}\n")).unwrap();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    runtime.block_on(collection.task(&home, member, Role::Solo)).unwrap();
    let questions = &collection.summary.tasks[0].questions.as_ref().unwrap().content;
    assert!(questions.find("questions-1.md").unwrap() < questions.find("questions-2.md").unwrap());
    assert!(questions.find("questions-2.md").unwrap() < questions.find("questions-10.md").unwrap());
    let tails = &collection.summary.gate[0].recent_output_tails;
    assert!(tails[0].contains("gate-10.log"), "{tails:?}");
    assert!(
        tails
            .iter()
            .all(|tail| tail.contains("modified:") && tail.contains("UTC")),
        "{tails:?}"
    );
}

#[test]
fn git_read_failures_are_prose_with_the_first_error_line() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&dir.path().to_str().unwrap().parse().unwrap());
    let team = team();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.summary.tasks.push(TaskSummaryFacts {
        id: team.crew().roster()[0].1.task().clone(),
        agent: "codex-1".parse().unwrap(),
        role: Role::Solo,
        status: team.crew().roster()[0].1.status(),
        run_count: 0,
        last_run_started: None,
        last_run_ended: None,
        last_activity: None,
        last_run_end_detail: None,
        questions: None,
    });
    runtime.block_on(collection.git(&home, &team));
    let failure = collection
        .assessment
        .unreadable
        .iter()
        .find(|part| part.part.starts_with("worktree"))
        .unwrap();
    assert!(failure.reason.contains("exited with 128: fatal:"), "{}", failure.reason);
    assert!(!failure.reason.contains("Exited {"));
    assert!(!failure.part.contains('"'));
}

#[test]
fn member_unpublished_origin_ref_does_not_trigger_origin_differs_anomaly() {
    use agent_core::config::AbsolutePath;
    use agent_core::diagnostic_assessment::{Anomaly, assess};
    use agent_core::state::TeamMembers;

    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());

    let team = Team::new(
        "team-20261117-120000-a1b2".parse().unwrap(),
        "example".parse().unwrap(),
        "2026-11-17T12:00:00Z".parse().unwrap(),
        MergeMode::Autopilot,
        TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Diagnose café\n".to_owned()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: Member::new(
                "task-20261117-120000-lea1".parse().unwrap(),
                Participant::Normal("codex-1".parse().unwrap()),
            ),
            members: vec![Member::new(
                "task-20261117-120000-mem1".parse().unwrap(),
                Participant::Normal("claude-1".parse().unwrap()),
            )],
            workers: vec![],
        })
        .unwrap(),
    );

    let lead_task = team.crew().roster()[0].1.task().clone();
    let member_task = team.crew().roster()[1].1.task().clone();

    let member_wt = home.worktree_dir(team.repo(), &member_task);
    std::fs::create_dir_all(&member_wt).unwrap();
    git(&member_wt, &["init", "--initial-branch=feature/member"]);
    git(&member_wt, &["commit", "--allow-empty", "-m", "Member work"]);

    let lead_wt = home.worktree_dir(team.repo(), &lead_task);
    std::fs::create_dir_all(&lead_wt).unwrap();
    git(&lead_wt, &["init", "--initial-branch=main"]);
    git(&lead_wt, &["commit", "--allow-empty", "-m", "Base"]);

    std::fs::write(
        home.config_file(),
        "[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    runtime.block_on(collection.observe(&home, &team, None)).unwrap();

    let containment_failure = collection
        .assessment
        .unreadable
        .iter()
        .find(|part| part.part == format!("git containment {member_task}"))
        .expect("member object is absent from the independent lead clone");
    assert!(
        containment_failure.reason.contains("exited with 128:"),
        "{}",
        containment_failure.reason
    );

    let lead_branch = collection
        .assessment
        .branches
        .iter()
        .find(|b| b.task == lead_task)
        .unwrap();
    assert!(matches!(lead_branch.origin, Some(Origin::Missing)));

    let member_branch = collection
        .assessment
        .branches
        .iter()
        .find(|b| b.task == member_task)
        .unwrap();
    assert!(member_branch.origin.is_none());

    let anomalies = assess(&collection.assessment);
    assert!(
        anomalies
            .iter()
            .any(|a| matches!(a, Anomaly::OriginDiffers(name) if name.as_str() == "main"))
    );
    assert!(
        !anomalies
            .iter()
            .any(|a| matches!(a, Anomaly::OriginDiffers(name) if name.as_str() == "feature/member"))
    );
}

fn setup_merged_team() -> Team {
    use agent_core::state::{PullRequest, TeamMembers};

    let team = Team::new(
        "team-20261117-120000-a1b2".parse().unwrap(),
        "example".parse().unwrap(),
        "2026-11-17T12:00:00Z".parse().unwrap(),
        MergeMode::Autopilot,
        TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Diagnose café\n".to_owned()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: Member::new(
                "task-20261117-120000-lea1".parse().unwrap(),
                Participant::Normal("codex-1".parse().unwrap()),
            ),
            members: vec![Member::new(
                "task-20261117-120000-mem1".parse().unwrap(),
                Participant::Normal("claude-1".parse().unwrap()),
            )],
            workers: vec![],
        })
        .unwrap(),
    );

    let round_id: agent_core::ids::RoundId = "round-20261117-120000-r111".parse().unwrap();
    let pr = PullRequest {
        number: agent_core::state::PrNumber::new(std::num::NonZeroU32::new(1).unwrap()),
        branch: "agents/claude-1/task-20261117-120000-mem1-1.1".parse().unwrap(),
        opened_at: "2026-11-17T12:02:00Z".parse().unwrap(),
    };
    let mut team_val = serde_json::to_value(&team).unwrap();
    team_val["items"][0]["phase"] = serde_json::json!({
        "phase": "done",
        "completion": {
            "how": "merged",
            "pr": pr,
            "at": "2026-11-17T12:05:00Z"
        }
    });
    team_val["rounds"] = serde_json::json!([
        {
            "id": round_id,
            "item": "1.1",
            "opened_at": "2026-11-17T12:01:00Z",
            "state": {
                "status": "closed",
                "at": "2026-11-17T12:05:00Z",
                "how": "merged"
            }
        }
    ]);
    serde_json::from_value(team_val).unwrap()
}

#[test]
fn merged_removed_worktree_falls_back_to_checkout() {
    use agent_core::config::AbsolutePath;

    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());

    let checkout_dir = tempfile::tempdir().unwrap();
    let checkout = checkout_dir.path();
    git(checkout, &["init", "--initial-branch=base"]);
    std::fs::write(checkout.join("file.txt"), "base content\n").unwrap();
    git(checkout, &["add", "file.txt"]);
    git(checkout, &["commit", "-m", "Base commit"]);

    git(checkout, &["branch", "agents/claude-1/task-20261117-120000-mem1-1.1"]);
    git(checkout, &["switch", "agents/claude-1/task-20261117-120000-mem1-1.1"]);
    std::fs::write(checkout.join("file.txt"), "member change\n").unwrap();
    git(checkout, &["add", "file.txt"]);
    git(checkout, &["commit", "-m", "Member commit"]);
    git(checkout, &["switch", "base"]);

    let team = setup_merged_team();
    let lead_task = team.crew().roster()[0].1.task().clone();
    let member_task = team.crew().roster()[1].1.task().clone();

    let lead_wt = home.worktree_dir(team.repo(), &lead_task);
    std::fs::create_dir_all(lead_wt.parent().unwrap()).unwrap();
    git(
        checkout,
        &[
            "worktree",
            "add",
            "-b",
            "agents/codex-1/task-20261117-120000-lea1",
            lead_wt.to_str().unwrap(),
            "base",
        ],
    );
    git(
        &lead_wt,
        &[
            "merge",
            "--no-ff",
            "agents/claude-1/task-20261117-120000-mem1-1.1",
            "-m",
            "Merge member",
        ],
    );

    std::fs::write(
        home.config_file(),
        "[repos.example]\nbase_branch = \"base\"\ngate = [\"true\"]\n",
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.checkout = Some(checkout.to_path_buf());
    collection.local_offset = time::UtcOffset::UTC;
    runtime.block_on(collection.observe(&home, &team, None)).unwrap();

    let member_git = collection
        .summary
        .git
        .iter()
        .find(|g| g.task_id == member_task)
        .unwrap();
    assert_eq!(
        member_git.branch.as_str(),
        "agents/claude-1/task-20261117-120000-mem1-1.1"
    );
    assert_eq!(member_git.commits_over_base.len(), 1);
    assert_eq!(member_git.commits_over_base[0].subject, "Member commit");
    assert_eq!(member_git.uncommitted_files, Vec::<PathBuf>::new());

    assert!(
        !collection
            .assessment
            .unreadable
            .iter()
            .any(|f| f.part.contains("task-20261117-120000-mem1")),
        "{:?}",
        collection.assessment.unreadable
    );

    let note = collection
        .artifacts
        .iter()
        .find(|(path, _)| path == &PathBuf::from("git").join("removed-worktrees.txt"));
    assert!(note.is_some());
    assert_eq!(
        collection.git_notice.as_deref(),
        Some(
            "Worktrees for merged rounds were removed; retained branches and commits were collected from the repository checkout."
        )
    );
}

#[test]
fn empty_team_history_records_history_artifacts_without_read_failure() {
    use agent_core::config::AbsolutePath;

    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = team();
    let task = team.crew().roster()[0].1.task().clone();
    let worktree = home.worktree_dir(team.repo(), &task);
    std::fs::create_dir_all(&worktree).unwrap();
    git(&worktree, &["init", "--initial-branch=main"]);
    git(&worktree, &["commit", "--allow-empty", "-m", "Base"]);
    std::fs::write(
        home.config_file(),
        "[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    runtime.block_on(collection.observe(&home, &team, None)).unwrap();

    assert_eq!(
        collection.summary.history_tail.as_deref(),
        Some("No history recorded for this team.")
    );
    assert!(
        !collection
            .assessment
            .unreadable
            .iter()
            .any(|f| f.part == "team history")
    );
    assert!(
        collection
            .artifacts
            .iter()
            .any(|(p, _)| p == &PathBuf::from("history.txt"))
    );
    assert!(
        collection
            .artifacts
            .iter()
            .any(|(p, _)| p == &PathBuf::from("history.jsonl"))
    );
}

#[test]
fn diagnostic_error_formats_git_process_status_and_stderr() {
    let err = ObservationError::Git {
        exit: ExitReport::Exited { code: 128 },
        stderr: b"fatal: bad object 1234\nextra details".to_vec(),
    };
    assert_eq!(
        diagnostic_error(&err),
        "read-only git command exited with 128: fatal: bad object 1234"
    );
}

#[test]
fn run_detail_formats_prose_with_local_offset() {
    use agent_core::state::RunEnd;
    use agent_core::status::RunOutcome;

    let state = RunState::Ended {
        at: "2026-11-17T12:00:00Z".parse().unwrap(),
        end: RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
    };
    let offset = time::UtcOffset::from_hms(1, 0, 0).unwrap();
    assert_eq!(
        run_detail_at(&state, offset),
        "ended, reported DONE, at 2026-11-17 12:00:00 UTC (2026-11-17 13:00:00 +01:00)"
    );
}

#[tokio::test]
async fn removed_worktree_does_not_substitute_another_tasks_member_branch() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = setup_merged_team();
    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "--initial-branch=base"]);
    git(checkout.path(), &["commit", "--allow-empty", "-m", "Base"]);
    git(
        checkout.path(),
        &["branch", "agents/claude-1/task-20261117-120000-zzzz-item"],
    );
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.checkout = Some(checkout.path().to_path_buf());
    collection.observe(&home, &team, None).await.unwrap();
    assert_eq!(collection.summary.git, vec![]);
}

#[tokio::test]
async fn worktree_metadata_failure_is_recorded_before_running_git() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = team();
    let task = team.crew().roster()[0].1.task();
    let path = home.worktree_dir(team.repo(), task);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&path, &path).unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.observe(&home, &team, None).await.unwrap();
    let failures: Vec<_> = collection
        .assessment
        .unreadable
        .iter()
        .filter(|part| part.part.starts_with("worktree metadata "))
        .collect();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].reason.contains("Too many levels of symbolic links"));
}

#[test]
fn retained_reference_records_reject_missing_fields() {
    assert_eq!(
        ref_fields("refs/heads/feature abc123").unwrap(),
        ("refs/heads/feature", "abc123")
    );
    for line in ["", "refs/heads/feature", " refs", "refs/heads/feature "] {
        assert!(matches!(ref_fields(line), Err(ObservationError::RefFields)), "{line}");
    }
}

#[test]
fn numbered_artifact_order_is_transitive_with_backups_and_leading_zeroes() {
    let paths = [
        "questions-2.md",
        "questions-10.md",
        "questions-10.md.bak",
        "questions-02.md",
    ];
    for left in paths.map(Path::new) {
        for middle in paths.map(Path::new) {
            for right in paths.map(Path::new) {
                if numbered_order(left, middle).is_lt() && numbered_order(middle, right).is_lt() {
                    assert!(numbered_order(left, right).is_lt(), "{left:?} < {middle:?} < {right:?}");
                }
            }
        }
    }
    assert_ne!(
        numbered_order(Path::new(paths[0]), Path::new(paths[3])),
        std::cmp::Ordering::Equal
    );
}

#[tokio::test]
async fn gate_source_timestamp_includes_the_machine_offset() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = team();
    let member = team.crew().roster()[0].1;
    let task_dir = home.task_dir(member.task());
    std::fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("gate-1.log");
    std::fs::write(&path, "gate passed\n").unwrap();
    let at: Timestamp = "2026-11-17T12:00:00Z".parse().unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::from(at.as_utc())))
        .unwrap();
    let mut collection = Collection::new(&team, at, Duration::from_secs(120)).unwrap();
    collection.local_offset = time::UtcOffset::from_hms(5, 30, 0).unwrap();
    collection.task(&home, member, Role::Solo).await.unwrap();
    let output = &collection.summary.gate[0].recent_output_tails[0];
    assert!(output.contains("+05:30"), "{output}");
    assert!(output.contains("12:00:00") && output.contains("17:30:00"), "{output}");
}

#[tokio::test]
async fn long_final_message_preserves_the_last_runs_reason_and_steps() {
    use agent_core::config::AbsolutePath;
    use agent_core::history::{AgentIdentity, Cut, Record, RunEndRecord, RunStart, RunWhy, SequencedRecord, Step};
    use agent_core::kinds::{CliKind, Tier};
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let mut team = team();
    let task = team.crew().roster()[0].1.task().clone();
    let at: Timestamp = "2026-11-17T12:00:00Z".parse().unwrap();
    let run: agent_core::ids::RunId = "run-20261117-120001-a1b2".parse().unwrap();
    team.begin_run(
        &task,
        agent_core::state::NewRun {
            id: run.clone(),
            at,
            session: None,
            rounds: vec![],
        },
    )
    .unwrap();
    let records = [
        Record::Run(Box::new(RunStart {
            run: run.clone(),
            team: team.id().clone(),
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
            why: RunWhy::WakeUp,
            prompt: "Please continue".to_owned(),
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
            end: RunEnd::Stopped,
            final_message: Some(format!("{} final café conclusion", "λ".repeat(150_000))),
        })),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, record)| SequencedRecord {
        sequence: u64::try_from(index).unwrap() + 1,
        at,
        team: team.id().clone(),
        rounds: vec![],
        record,
        cut: Cut::default(),
    })
    .collect::<Vec<_>>();
    let mut collection = Collection::new(&team, at, Duration::from_secs(120)).unwrap();
    collection
        .task(&home, team.crew().roster()[0].1, Role::Solo)
        .await
        .unwrap();
    collection.enrich_last_run_details(&records, &team);
    let summary = agent_core::debug_summary::render_command_summary(
        &collection.summary,
        time::UtcOffset::UTC,
        &[],
        agent_core::debug_summary::UntrackedCollection::Deferred,
        str::to_owned,
    )
    .unwrap();
    assert!(
        summary.contains("Recorded café step"),
        "the step must survive the final-message budget"
    );
    assert!(
        summary.contains("a message or a board item woke the task"),
        "the reason must survive the final-message budget"
    );
    assert!(summary.contains("final café conclusion"));
}

#[tokio::test]
async fn retained_member_branch_uses_the_recorded_merged_item_not_lexical_order() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "--initial-branch=base"]);
    git(checkout.path(), &["commit", "--allow-empty", "-m", "Base"]);
    let recorded = "agents/claude-1/task-20261117-120000-mem1-1.10";
    for branch in [recorded, "agents/claude-1/task-20261117-120000-mem1-1.9"] {
        git(checkout.path(), &["branch", branch]);
    }
    let mut value = serde_json::to_value(setup_merged_team()).unwrap();
    value["items"][0]["phase"]["completion"]["pr"]["branch"] = serde_json::json!(recorded);
    let team: Team = serde_json::from_value(value).unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.checkout = Some(checkout.path().to_owned());
    for (role, member) in team.crew().roster() {
        collection.task(&home, member, role).await.unwrap();
    }
    collection.git(&home, &team).await;
    assert_eq!(
        collection
            .summary
            .git
            .iter()
            .find(|facts| facts.task_id == *team.crew().roster()[1].1.task())
            .unwrap()
            .branch
            .as_str(),
        recorded
    );
}

#[tokio::test]
async fn earlier_merged_tasks_stay_readable_after_a_later_round_opens() {
    use agent_core::config::AbsolutePath;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "--initial-branch=base"]);
    git(checkout.path(), &["commit", "--allow-empty", "-m", "Base"]);
    git(
        checkout.path(),
        &["branch", "agents/claude-1/task-20261117-120000-mem1-1.1"],
    );
    let mut value = serde_json::to_value(setup_merged_team()).unwrap();
    value["task_list"] = serde_json::json!("Build.\n\n1. Feature\n[ ] 1.1 Diagnose café\n[ ] 1.2 Next item\n");
    value["items"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id":"1.2", "phase":{"phase":"pending"}}));
    let mut team: Team = serde_json::from_value(value).unwrap();
    team.open_round(
        "round-20261117-121000-r222".parse().unwrap(),
        Some("1.2".parse().unwrap()),
        "2026-11-17T12:10:00Z".parse().unwrap(),
    )
    .unwrap();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:11:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.checkout = Some(checkout.path().to_owned());
    for (role, member) in team.crew().roster() {
        collection.task(&home, member, role).await.unwrap();
    }
    collection.git(&home, &team).await;
    assert!(
        collection
            .summary
            .git
            .iter()
            .any(|facts| facts.task_id == *team.crew().roster()[1].1.task()),
        "the earlier merged member must remain readable"
    );
}

#[test]
fn general_collection_failure_has_no_machine_destination() {
    let team = team();
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    collection.failed("agent status", "daemon unavailable");
    assert_eq!(
        collection.assessment.unreadable[0].section,
        agent_core::debug_summary::SummarySection::CollectionFailures
    );
}

#[tokio::test]
async fn artifact_failures_use_the_sections_of_the_observed_parts() {
    use agent_core::config::AbsolutePath;
    use agent_core::debug_summary::SummarySection;
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    let team = team();
    let member = team.crew().roster()[0].1;
    let mut collection =
        Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    std::fs::create_dir_all(home.task_dir(member.task())).unwrap();
    std::fs::write(home.gate_record_file(member.task()), b"invalid JSON").unwrap();
    collection.gate(&home, member.task()).await;
    assert_eq!(
        collection.assessment.unreadable.last().unwrap().section,
        SummarySection::GateRecords
    );
    collection
        .tree(dir.path().join("missing"), PathBuf::from("tasks"), member)
        .await;
    assert_eq!(
        collection.assessment.unreadable.last().unwrap().section,
        SummarySection::TaskDetails
    );
    collection.task(&home, member, Role::Solo).await.unwrap();
    collection.git(&home, &team).await;
    assert_eq!(
        collection.assessment.unreadable.last().unwrap().section,
        SummarySection::GitWorktrees
    );
}
