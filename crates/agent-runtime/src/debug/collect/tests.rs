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
