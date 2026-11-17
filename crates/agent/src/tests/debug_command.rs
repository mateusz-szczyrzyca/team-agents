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
    std::fs::create_dir_all(&lead_wt).unwrap();
    std::fs::create_dir_all(&member_wt).unwrap();

    run_git(&lead_wt, &["init", "-q", "-b", "main"]);
    run_git(
        &lead_wt,
        &["commit", "-q", "--allow-empty", "-m", "initial lead commit"],
    );

    run_git(&member_wt, &["init", "-q", "-b", "agents/claude-1/item-1-1"]);
    run_git(&member_wt, &["commit", "-q", "--allow-empty", "-m", "member work"]);

    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    std::fs::write(home.secrets_file(), "").unwrap();
    std::fs::set_permissions(home.secrets_file(), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(home.config_file(), "").unwrap();

    (dir, home, state, team, lead_task, member_task, round)
}

fn find_summary(out_dir: &Path) -> Option<PathBuf> {
    if out_dir.join("SUMMARY.md").exists() {
        return Some(out_dir.join("SUMMARY.md"));
    }
    if let Ok(entries) = std::fs::read_dir(out_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("SUMMARY.md").exists() {
                return Some(path.join("SUMMARY.md"));
            }
        }
    }
    None
}

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_files(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
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

    let summary_path = find_summary(out_dir.path()).expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(
        summary.contains("failed") || summary.contains("Failed"),
        "summary was: {summary}"
    );
    assert!(
        summary.contains("questions-1.md") || summary.contains("unanswered") || summary.contains("Questions"),
        "summary was: {summary}"
    );
}

#[test]
fn debug_done_lead_lacking_unfinished_member_branch() {
    let (_dir, home, _state, team, _lead_task, member_task, _round) = setup_test_home();
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

    let summary_path = find_summary(out_dir.path()).expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(
        summary.contains("branch") || summary.contains("Branch"),
        "summary was: {summary}"
    );
}

#[test]
fn debug_removed_worktree_reports_read_failure_and_rest_continues() {
    let (_dir, home, _state, team, _lead_task, member_task, _round) = setup_test_home();
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

    let summary_path = find_summary(out_dir.path()).expect("SUMMARY.md should exist");
    let summary = std::fs::read_to_string(&summary_path).unwrap();
    assert!(
        summary.contains("unreadable") || summary.contains("failed") || summary.contains("Unreadable"),
        "summary was: {summary}"
    );
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
            "--daemon-log",
            daemon_log.to_str().unwrap(),
            "--out",
            out_dir.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, ExitCode::SUCCESS, "stderr was: {err}");

    let all_files = collect_files(out_dir.path());
    assert!(!all_files.is_empty(), "debug bundle should produce files");
    for file in all_files {
        if let Ok(content) = std::fs::read_to_string(&file) {
            assert!(
                !content.contains(secret_token),
                "file {} contained unredacted secret!",
                file.display()
            );
        }
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

    let entries: Vec<_> = std::fs::read_dir(&bundle_parent).unwrap().flatten().collect();
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
    let lead_git_head_before = std::fs::read(lead_wt.join(".git/HEAD")).unwrap();
    let member_git_head_before = std::fs::read(member_wt.join(".git/HEAD")).unwrap();

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

    let lead_git_head_after = std::fs::read(lead_wt.join(".git/HEAD")).unwrap();
    assert_eq!(
        lead_git_head_before, lead_git_head_after,
        "git HEAD must not be changed"
    );

    let member_git_head_after = std::fs::read(member_wt.join(".git/HEAD")).unwrap();
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
