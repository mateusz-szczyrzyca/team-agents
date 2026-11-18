//! `agent debug` on the command line: parsing, validation, offline fallback, and end-to-end execution.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agent_core::ids::{RoundId, TaskId, TeamId};

use super::*;

fn environment(home: &Path) -> Environment {
    Environment::new(Some(OsString::from(home)))
}

fn agent(environment: &Environment, args: &[&str]) -> (ExitCode, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_in(
        environment,
        std::iter::once("agent").chain(args.iter().copied()),
        &mut out,
        &mut err,
    )
    .unwrap();
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

use crate::fixtures::{at, repo, task, team_id, team_state};
use agent_core::config::AbsolutePath;
use agent_core::ids::ItemId;
use agent_core::state::State;
use agent_core::status::TaskStatus;
use agent_runtime::store::Home;

#[test]
fn debug_requires_target_argument() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = agent(&environment(dir.path()), &["debug"]);

    assert_eq!(code, ExitCode::from(USAGE_ERROR));
    assert_eq!(out, "");
    assert!(
        err.contains("--round") || err.contains("--team") || err.contains("--task"),
        "{err}"
    );
}

#[test]
fn debug_rejects_conflicting_targets() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = agent(
        &environment(dir.path()),
        &[
            "debug",
            "--round",
            "round-20261117-230000-aaaa",
            "--team",
            "team-20261117-230000-bbbb",
        ],
    );

    assert_eq!(code, ExitCode::from(USAGE_ERROR));
    assert_eq!(out, "");
    assert!(err.contains("cannot be used with"), "{err}");
}

#[test]
fn debug_parses_round_target() {
    let round: RoundId = "round-20261117-230000-q1w2".parse().unwrap();
    let invocation = parse_invocation(["agent", "debug", "--round", "round-20261117-230000-q1w2"]).unwrap();

    let Invocation::Debug(args) = invocation else {
        panic!("expected Debug invocation");
    };
    assert_eq!(args.round, Some(round));
    assert_eq!(args.team, None);
    assert_eq!(args.task, None);
}

#[test]
fn debug_parses_team_target() {
    let team: TeamId = "team-20261117-230000-t1e2".parse().unwrap();
    let invocation = parse_invocation(["agent", "debug", "--team", "team-20261117-230000-t1e2"]).unwrap();

    let Invocation::Debug(args) = invocation else {
        panic!("expected Debug invocation");
    };
    assert_eq!(args.round, None);
    assert_eq!(args.team, Some(team));
    assert_eq!(args.task, None);
}

#[test]
fn debug_parses_task_target() {
    let task: TaskId = "task-20261117-230000-a1b2".parse().unwrap();
    let invocation = parse_invocation(["agent", "debug", "--task", "task-20261117-230000-a1b2"]).unwrap();

    let Invocation::Debug(args) = invocation else {
        panic!("expected Debug invocation");
    };
    assert_eq!(args.round, None);
    assert_eq!(args.team, None);
    assert_eq!(args.task, Some(task));
}

#[test]
fn debug_parses_all_optional_flags() {
    let team: TeamId = "team-20261117-230000-t1e2".parse().unwrap();
    let invocation = parse_invocation([
        "agent",
        "debug",
        "--team",
        "team-20261117-230000-t1e2",
        "--out",
        "/tmp/debug-out",
        "--stale-minutes",
        "30",
        "--daemon-log",
        "/tmp/daemon.log",
    ])
    .unwrap();

    let Invocation::Debug(args) = invocation else {
        panic!("expected Debug invocation");
    };
    assert_eq!(args.team, Some(team));
    assert_eq!(args.out, Some(PathBuf::from("/tmp/debug-out")));
    assert_eq!(args.stale_minutes, Some(30));
    assert_eq!(args.daemon_log, Some(PathBuf::from("/tmp/daemon.log")));
}

#[test]
fn debug_offline_execution_succeeds_without_daemon() {
    let (_dir, home, _state, team, _lead_task, _member_task, _round) = setup_test_home();
    let (code, out, err) = agent(
        &environment(home.state_file().parent().unwrap()),
        &["debug", "--team", team.as_str()],
    );

    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");
    assert!(out.contains("Debug summary:"), "stdout was: {out}");
    let summary_path = find_summary(&home.root().join("debug")).unwrap().unwrap();
    let summary = std::fs::read_to_string(summary_path).unwrap();
    assert!(
        summary.contains("Could not collect agent status (daemon does not answer):"),
        "{summary}"
    );
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Fake",
            "-c",
            "user.email=fake@example.com",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git failed: {args:?}");
}

fn run_git_capture(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Fake",
            "-c",
            "user.email=fake@example.com",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git failed: {args:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn setup_test_home() -> (tempfile::TempDir, Home, State, TeamId, TaskId, TaskId, RoundId) {
    let dir = tempfile::tempdir().unwrap();
    let abs_path: AbsolutePath = dir.path().to_str().unwrap().parse().unwrap();
    let home = Home::new(&abs_path);
    let mut state = team_state();
    let team = team_id(0);
    let lead_task = task(1);
    let member_task = task(2);
    let round: RoundId = "round-20261111-153001-k3x9".parse().unwrap();
    let item_id: ItemId = "1.1".parse().unwrap();
    state.open_round(&team, round.clone(), Some(item_id), at(1)).unwrap();

    std::fs::create_dir_all(home.tasks_dir()).unwrap();
    std::fs::create_dir_all(home.task_dir(&lead_task)).unwrap();
    std::fs::create_dir_all(home.task_dir(&member_task)).unwrap();

    let repo_dir = home.worktrees_dir().join(repo().as_str());
    std::fs::create_dir_all(&repo_dir).unwrap();
    let lead_wt = home.worktree_dir(&repo(), &lead_task);
    let member_wt = home.worktree_dir(&repo(), &member_task);

    run_git(&repo_dir, &["init", "-q", "-b", "main", lead_wt.to_str().unwrap()]);
    run_git(
        &lead_wt,
        &["commit", "-q", "--allow-empty", "-m", "initial lead commit"],
    );

    run_git(
        &lead_wt,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "agents/claude-1/item-1-1",
            member_wt.to_str().unwrap(),
        ],
    );
    run_git(&member_wt, &["commit", "-q", "--allow-empty", "-m", "member work"]);

    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    std::fs::write(home.secrets_file(), "").unwrap();
    std::fs::set_permissions(home.secrets_file(), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(home.config_file(), "").unwrap();

    (dir, home, state, team, lead_task, member_task, round)
}

fn git_head_path(worktree: &Path) -> PathBuf {
    let git_path = worktree.join(".git");
    if git_path.is_dir() {
        git_path.join("HEAD")
    } else {
        let content = std::fs::read_to_string(&git_path).unwrap();
        let gitdir = content
            .lines()
            .find_map(|line| line.strip_prefix("gitdir: "))
            .unwrap()
            .trim();
        PathBuf::from(gitdir).join("HEAD")
    }
}

fn find_summary(out_dir: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    if out_dir.join("SUMMARY.md").exists() {
        return Ok(Some(out_dir.join("SUMMARY.md")));
    }
    for entry in std::fs::read_dir(out_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("SUMMARY.md").exists() {
            return Ok(Some(path.join("SUMMARY.md")));
        }
    }
    Ok(None)
}

fn collect_files(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

#[test]
fn debug_round_team_and_task_resolve_same_team() {
    let (_dir, home, _state, team, lead_task, member_task, round) = setup_test_home();
    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();

    let (code1, out1, err1) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code1, ExitCode::SUCCESS, "stderr was: {err1}");
    assert!(out1.contains("Debug summary:"), "stdout was: {out1}");

    let (code2, out2, err2) = agent(
        &env,
        &[
            "debug",
            "--round",
            round.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code2, ExitCode::SUCCESS, "stderr was: {err2}");
    assert!(out2.contains("Debug summary:"), "stdout was: {out2}");

    let (code3, out3, err3) = agent(
        &env,
        &[
            "debug",
            "--task",
            lead_task.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code3, ExitCode::SUCCESS, "stderr was: {err3}");
    assert!(out3.contains("Debug summary:"), "stdout was: {out3}");

    let (code4, out4, err4) = agent(
        &env,
        &[
            "debug",
            "--task",
            member_task.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code4, ExitCode::SUCCESS, "stderr was: {err4}");
    assert!(out4.contains("Debug summary:"), "stdout was: {out4}");
    for output in [&out1, &out2, &out3, &out4] {
        let path = output
            .lines()
            .find_map(|line| line.strip_prefix("Debug summary: "))
            .unwrap();
        let summary = std::fs::read_to_string(path).unwrap();
        assert!(
            summary.contains(&format!("Team `{team}` (repository `{}`)", repo())),
            "{summary}"
        );
    }
}

#[test]
fn debug_failed_task_and_unanswered_questions() {
    let (_dir, home, mut state, team, _lead_task, member_task, _round) = setup_test_home();
    let (_role, mut found) = state.find_task_mut(&member_task).unwrap();
    found.transition(TaskStatus::Running).unwrap();
    found.transition(TaskStatus::Failed).unwrap();
    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();

    let question_file = home.task_dir(&member_task).join("questions-1.md");
    std::fs::write(
        &question_file,
        "# Questions — task\n\n## 1. Proceed?\na) Yes\nb) No\n\n**Recommended:** a)\n\n**Verdict:**\n",
    )
    .unwrap();

    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let (code, out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");
    assert!(out.contains("Debug summary:"), "stdout was: {out}");

    let summary_path = find_summary(out_dir.path()).unwrap().expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(summary.contains(&format!("Task {member_task} is failed.")), "{summary}");
    assert!(
        summary.contains(&format!(
            "Questions file {:?} has no answer yet.",
            question_file.as_os_str()
        )),
        "{summary}"
    );
}

#[test]
fn debug_done_lead_lacking_unfinished_member_branch() {
    let (_dir, home, mut state, team, lead_task, member_task, _round) = setup_test_home();
    let (_role, mut lead) = state.find_task_mut(&lead_task).unwrap();
    lead.transition(TaskStatus::Running).unwrap();
    lead.transition(TaskStatus::Done).unwrap();
    state
        .team_mut(&team)
        .unwrap()
        .item_mut("1.1".parse().unwrap())
        .unwrap()
        .advance(agent_core::state::ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/item-1-1".parse().unwrap(),
        })
        .unwrap();
    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    let member_wt = home.worktree_dir(&repo(), &member_task);
    std::fs::write(member_wt.join("unmerged.txt"), "unmerged content").unwrap();
    run_git(&member_wt, &["add", "unmerged.txt"]);
    run_git(&member_wt, &["commit", "-q", "-m", "unmerged member commit"]);

    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    let summary_path = find_summary(out_dir.path()).unwrap().expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(
        summary.contains("The lead branch does not contain assigned unfinished branch agents/claude-1/item-1-1."),
        "summary was: {summary}"
    );
}

#[test]
fn debug_removed_worktree_reports_read_failure_and_rest_continues() {
    let (_dir, home, _state, team, lead_task, member_task, _round) = setup_test_home();
    let member_wt = home.worktree_dir(&repo(), &member_task);
    std::fs::remove_dir_all(&member_wt).unwrap();

    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    let summary_path = find_summary(out_dir.path()).unwrap().expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(
        summary.contains(&format!("Could not collect worktree {:?}:", member_wt.as_os_str())),
        "summary was: {summary}"
    );
    let bundle = summary_path.parent().unwrap();
    assert!(bundle.join("git").join(format!("{lead_task}.txt")).is_file());
    assert!(!bundle.join("git").join(format!("{member_task}.txt")).exists());
    assert!(summary.contains(&format!("### Task `{lead_task}`")), "{summary}");
}

#[test]
fn debug_redaction_secrets_prompt_raw_stdout_and_env_values() {
    let (_dir, home, _state, team, _lead_task, member_task, _round) = setup_test_home();
    let secret_token = "ghp_supersecret1234567890abcdef";
    let secrets_content =
        format!("github_token = \"{secret_token}\"\n[agent_env]\nAPI_KEY = \"sk-supersecret9876543210\"\n");
    std::fs::write(home.secrets_file(), secrets_content).unwrap();
    std::fs::set_permissions(home.secrets_file(), std::fs::Permissions::from_mode(0o600)).unwrap();

    let prompt_file = home.task_dir(&member_task).join("prompt.md");
    std::fs::write(&prompt_file, format!("Prompt with {secret_token} inside")).unwrap();

    let raw_file = home.task_dir(&member_task).join("activity.jsonl");
    std::fs::write(&raw_file, format!("{{\"text\":\"output with {secret_token}\"}}\n")).unwrap();

    let daemon_log = home.state_file().parent().unwrap().join("daemon.log");
    std::fs::write(&daemon_log, format!("log with {secret_token}\n")).unwrap();

    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    let all_files = collect_files(out_dir.path()).unwrap();
    assert!(!all_files.is_empty(), "debug bundle should produce files");
    for file in all_files {
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(
            !content.contains(secret_token),
            "file {} contained unredacted secret!",
            file.display()
        );
    }
}

#[test]
fn debug_strict_missing_secrets_no_output_directory() {
    let (_dir, home, _state, team, _lead_task, _member_task, _round) = setup_test_home();
    std::fs::remove_file(home.secrets_file()).unwrap();

    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let bundle_parent = out_dir.path().join("bundle_parent");
    std::fs::create_dir_all(&bundle_parent).unwrap();

    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            bundle_parent.to_str().unwrap(),
        ],
    );
    assert_ne!(code, ExitCode::SUCCESS);
    assert!(err.contains("secrets") || err.contains("Secrets"), "stderr was: {err}");

    let entries: Vec<_> = std::fs::read_dir(&bundle_parent)
        .unwrap()
        .map(|res| res.unwrap().path())
        .collect();
    assert!(
        entries.is_empty(),
        "no bundle directory should be created when secrets are unreadable"
    );
}

#[test]
fn debug_home_state_tasks_and_git_refs_unchanged() {
    let (_dir, home, _state, team, lead_task, member_task, _round) = setup_test_home();
    let env = environment(home.state_file().parent().unwrap());

    let state_before = std::fs::read(home.state_file()).unwrap();
    let lead_wt = home.worktree_dir(&repo(), &lead_task);
    let member_wt = home.worktree_dir(&repo(), &member_task);
    let lead_git_head_before = std::fs::read(git_head_path(&lead_wt)).unwrap();
    let member_git_head_before = std::fs::read(git_head_path(&member_wt)).unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    let state_after = std::fs::read(home.state_file()).unwrap();
    assert_eq!(
        state_before, state_after,
        "state.json must not be modified by agent debug"
    );

    let lead_git_head_after = std::fs::read(git_head_path(&lead_wt)).unwrap();
    assert_eq!(
        lead_git_head_before, lead_git_head_after,
        "git HEAD must not be changed"
    );

    let member_git_head_after = std::fs::read(git_head_path(&member_wt)).unwrap();
    assert_eq!(
        member_git_head_before, member_git_head_after,
        "git HEAD must not be changed"
    );
}

#[tokio::test]
async fn debug_online_daemon_status_reuses_status_rendering() {
    let (_dir, home, state, team, _lead_task, _member_task, _round) = setup_test_home();
    let teams = vec![agent_core::protocol::TeamSummary::from(&state.teams()[0])];
    let listed = teams.clone();
    let _daemon = crate::testing::FakeDaemon::serve(&home.socket_file(), move |request| {
        crate::testing::Answer::Lines(vec![
            agent_core::protocol::Frame::Response(agent_core::protocol::Response::ok(
                request.id,
                agent_core::protocol::Reply::Teams { teams: listed.clone() },
            ))
            .to_line()
            .unwrap(),
        ])
    });

    let status = crate::commands::debug::query_daemon_status(&home, &Target::Team(team))
        .await
        .unwrap()
        .unwrap();
    let expected = crate::commands::status::render(&teams);
    assert_eq!(status, expected);
}

#[test]
fn debug_done_lead_with_uncontained_member_branch_reports_exact_unmerged_anomaly() {
    let (_dir, home, mut state, team, lead_task, member_task, _round) = setup_test_home();

    // Transition lead task to Done
    let (_role, mut lead_found) = state.find_task_mut(&lead_task).unwrap();
    lead_found.transition(TaskStatus::Running).unwrap();
    lead_found.transition(TaskStatus::Done).unwrap();

    // Advance item 1.1 to InProgress on member's branch
    let item_id: ItemId = "1.1".parse().unwrap();
    state
        .team_mut(&team)
        .unwrap()
        .item_mut(item_id)
        .unwrap()
        .advance(agent_core::state::ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/item-1-1".parse().unwrap(),
        })
        .unwrap();

    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();

    // Member worktree creates an unmerged commit not contained in lead branch
    let member_wt = home.worktree_dir(&repo(), &member_task);
    std::fs::write(member_wt.join("unmerged_feature.txt"), "feature content").unwrap();
    run_git(&member_wt, &["add", "unmerged_feature.txt"]);
    run_git(&member_wt, &["commit", "-q", "-m", "unmerged feature commit"]);

    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    let summary_path = find_summary(out_dir.path()).unwrap().expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(
        summary.contains("The lead branch does not contain assigned unfinished branch agents/claude-1/item-1-1."),
        "summary was: {summary}"
    );
}

#[test]
fn debug_rejects_unknown_target_with_exit_3_and_creates_no_directory() {
    let (_dir, home, _state, _team, _lead_task, _member_task, _round) = setup_test_home();
    let env = environment(home.state_file().parent().unwrap());
    let out_dir = tempfile::tempdir().unwrap();
    let bundle_parent = out_dir.path().join("bundle_parent");
    std::fs::create_dir_all(&bundle_parent).unwrap();

    let unknown_team = "team-20990101-000000-zzzz";
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            unknown_team,
            "--out",
            bundle_parent.to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::from(3), "stderr was: {err}");
    assert!(err.contains(unknown_team), "stderr was: {err}");

    let entries: Vec<_> = std::fs::read_dir(&bundle_parent)
        .unwrap()
        .map(|res| res.unwrap().path())
        .collect();
    assert!(
        entries.is_empty(),
        "no bundle directory should be created when target is unknown"
    );
}

#[test]
fn debug_snapshots_all_task_files_and_git_refs_intact() {
    let (_dir, home, _state, team, lead_task, member_task, _round) = setup_test_home();
    let env = environment(home.state_file().parent().unwrap());

    // Populate task artifacts for both lead and member
    let lead_dir = home.task_dir(&lead_task);
    std::fs::write(lead_dir.join("prompt.md"), "lead prompt text").unwrap();
    std::fs::write(lead_dir.join("activity.jsonl"), "{\"lead\":\"event\"}\n").unwrap();
    std::fs::write(lead_dir.join("meta.env"), "TASK_ID=lead\n").unwrap();

    let member_dir = home.task_dir(&member_task);
    std::fs::write(member_dir.join("prompt.md"), "member prompt text").unwrap();
    std::fs::write(member_dir.join("activity.jsonl"), "{\"member\":\"event\"}\n").unwrap();
    std::fs::write(member_dir.join("final-message.txt"), "Done item 1.1\n").unwrap();

    let lead_wt = home.worktree_dir(&repo(), &lead_task);
    let member_wt = home.worktree_dir(&repo(), &member_task);

    // Record git refs before debug runs
    let lead_refs_before = run_git_capture(&lead_wt, &["show-ref"]);
    let member_refs_before = run_git_capture(&member_wt, &["show-ref"]);

    let out_dir = tempfile::tempdir().unwrap();
    let (code, _out, err) = agent(
        &env,
        &[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    // Verify git refs remain completely untouched
    let lead_refs_after = run_git_capture(&lead_wt, &["show-ref"]);
    let member_refs_after = run_git_capture(&member_wt, &["show-ref"]);
    assert_eq!(lead_refs_before, lead_refs_after, "lead git refs must not be modified");
    assert_eq!(
        member_refs_before, member_refs_after,
        "member git refs must not be modified"
    );

    // Verify bundle contains snapshots of task files
    let all_files = collect_files(out_dir.path()).unwrap();
    assert!(
        all_files
            .iter()
            .any(|f| f.ends_with("lead/prompt.md") || f.ends_with(format!("{}/prompt.md", lead_task.as_str()))),
        "bundle must snapshot lead task prompt"
    );
    assert!(
        all_files
            .iter()
            .any(|f| f.ends_with("member/prompt.md") || f.ends_with(format!("{}/prompt.md", member_task.as_str()))),
        "bundle must snapshot member task prompt"
    );
    assert!(
        all_files.iter().any(|f| f.ends_with("final-message.txt")),
        "bundle must snapshot member final-message.txt"
    );
}

#[tokio::test]
async fn debug_status_preserves_the_daemons_history_health_line() {
    let (_dir, home, state, team, _, _, _) = setup_test_home();
    let teams = vec![agent_core::protocol::TeamSummary::from(&state.teams()[0])];
    let health = agent_core::history::HistoryHealth {
        state: agent_core::history::HistoryState::Disabled,
        lost: 0,
    };
    let expected = format!(
        "{}{}",
        crate::commands::status::render(&teams),
        crate::commands::status::render_history(Some(&health))
    );
    let _daemon = crate::testing::FakeDaemon::serve(&home.socket_file(), move |request| {
        crate::testing::Answer::Lines(vec![
            agent_core::protocol::Frame::Response(agent_core::protocol::Response::ok(
                request.id,
                agent_core::protocol::Reply::Overview {
                    teams: teams.clone(),
                    history: health.clone(),
                },
            ))
            .to_line()
            .unwrap(),
        ])
    });
    let actual = crate::commands::debug::query_daemon_status(&home, &Target::Team(team))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn debug_reports_unreadable_explicit_daemon_log_in_its_bundle() {
    let (_dir, home, _state, team, _lead, _member, _round) = setup_test_home();
    let output = tempfile::tempdir().unwrap();
    let parent = output.path().join("diagnostics");
    let missing = output.path().join("missing-daemon.log");
    let (code, out, err) = agent(
        &environment(home.root()),
        &[
            "debug",
            "--team",
            team.as_str(),
            "--daemon-log",
            missing.to_str().unwrap(),
            "--out",
            parent.to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "{err}");
    assert_eq!(err, "");
    assert!(out.contains("Debug summary:"), "{out}");
    let summary = std::fs::read_to_string(find_summary(&parent).unwrap().unwrap()).unwrap();
    let log = summary
        .split(agent_core::debug_summary::SummarySection::DaemonLog.heading())
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert!(log.contains("Could not read daemon log:"), "{log}");
    assert!(log.contains(missing.to_str().unwrap()), "{log}");
    assert!(!log.contains("not found."), "{log}");
}

#[test]
fn debug_collects_live_history_from_active_wal_without_modifying_home() {
    use agent_runtime::history::Store;

    let (_dir, home, _state, team, _lead, member, _round) = setup_test_home();
    let history_file = home.history_file();
    let at = "2026-11-18T06:00:00Z".parse().unwrap();
    let run: agent_core::ids::RunId = "run-20261118-060000-a1b2".parse().unwrap();
    let task = member;
    let mut database =
        agent_runtime::history::HistoryDb::open(&history_file, agent_core::history::MaxTextBytes::default()).unwrap();
    database
        .write(&[
            agent_core::history::Record::Team(Box::new(agent_core::history::TeamStart {
                team: team.clone(),
                at,
                repo: "example".to_owned(),
                base_branch: "main".parse().unwrap(),
                merge: agent_core::state::MergeMode::NoMerge,
                task_list: "Live task list.".to_owned(),
                plan: Vec::new(),
            })),
            agent_core::history::Record::Run(Box::new(agent_core::history::RunStart {
                run: run.clone(),
                team: team.clone(),
                task,
                at,
                agent: agent_core::history::AgentIdentity {
                    name: "codex-1".to_owned(),
                    tier: agent_core::kinds::Tier::Normal,
                    cli: agent_core::kinds::CliKind::Codex,
                    model: None,
                },
                role: agent_core::kinds::Role::Member,
                rounds: vec![],
                why: agent_core::history::RunWhy::FirstRun,
                prompt: "Active WAL live prompt".to_owned(),
                worktree: agent_core::config::AbsolutePath::try_from("/tmp/worktree".to_owned()).unwrap(),
                branch: "feature".parse().unwrap(),
                session: None,
            })),
            agent_core::history::Record::Step(Box::new(agent_core::history::Step {
                run: run.clone(),
                number: std::num::NonZeroU32::MIN,
                at,
                event: agent_core::cli::ActivityEvent::Text("Active WAL live step".to_owned()),
            })),
            agent_core::history::Record::RunEnd(Box::new(agent_core::history::RunEndRecord {
                run,
                at,
                end: agent_core::state::RunEnd::Stopped,
                final_message: Some("Active WAL live final message".to_owned()),
            })),
        ])
        .unwrap();

    let wal = home.root().join("history.db-wal");
    assert!(wal.exists());
    let before_db = std::fs::read(&history_file).unwrap();
    let before_wal = std::fs::read(&wal).unwrap();

    let output = tempfile::tempdir().unwrap();
    let parent = output.path().join("diagnostics");
    let (code, out, err) = agent(
        &environment(home.root()),
        &["debug", "--team", team.as_str(), "--out", parent.to_str().unwrap()],
    );
    assert_eq!(code, ExitCode::from(0), "{err}");
    assert!(out.contains("Debug summary:"), "{out}");

    assert_eq!(std::fs::read(&history_file).unwrap(), before_db);
    assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
    drop(database);

    let summary_path = parent
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("SUMMARY.md");
    let summary = std::fs::read_to_string(summary_path).unwrap();
    assert!(summary.contains("Active WAL live prompt"), "{summary}");
    assert!(summary.contains("Active WAL live final message"), "{summary}");
}
