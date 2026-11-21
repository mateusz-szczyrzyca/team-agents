use std::fs;

use agent_core::ids::RunId;
use agent_core::plan::judge::JudgementState;
use agent_core::plan::parse_plan;
use tempfile::TempDir;

use super::{CheckContext, execute_step_check};

fn parse_first_step(step_text: &str) -> agent_core::plan::Step {
    let full_text = format!("1. Feature\n{step_text}");
    let plan = parse_plan(&full_text).expect("valid plan");
    plan.steps().next().expect("at least one step").clone()
}

struct TestEnv {
    _dir: TempDir,
    worktree: std::path::PathBuf,
    task_dir: std::path::PathBuf,
    run: RunId,
}

impl TestEnv {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let worktree = dir.path().join("worktree");
        let task_dir = dir.path().join("task");
        fs::create_dir_all(&worktree).expect("create worktree");
        fs::create_dir_all(&task_dir).expect("create task_dir");
        let run = "run-20261121-010000-abcd".parse::<RunId>().expect("run id");
        Self {
            _dir: dir,
            worktree,
            task_dir,
            run,
        }
    }

    fn context<'a>(&'a self, seconds: u64, refuse: &'a [String]) -> CheckContext<'a> {
        CheckContext {
            worktree: &self.worktree,
            task_dir: &self.task_dir,
            run: Some(&self.run),
            check_seconds: seconds,
            check_refuse: refuse,
            envs: &[],
        }
    }
}

#[tokio::test]
async fn check_exists_confirms_when_file_exists() {
    let env = TestEnv::new();
    let file_path = env.worktree.join("docs/guide.md");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, "hello").unwrap();

    let step = parse_first_step("[x] 1.1 First item\n    Check: exists docs/guide.md\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Confirmed);
    assert_eq!(result.check_line.as_deref(), Some("exists docs/guide.md"));
    assert!(result.reason.is_none());
}

#[tokio::test]
async fn check_exists_confirms_when_directory_exists() {
    let env = TestEnv::new();
    let dir_path = env.worktree.join("docs/images");
    fs::create_dir_all(&dir_path).unwrap();

    let step = parse_first_step("[x] 1.1 Images\n    Check: exists docs/images\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Confirmed);
    assert_eq!(result.check_line.as_deref(), Some("exists docs/images"));
    assert!(result.reason.is_none());
}

#[tokio::test]
async fn check_exists_mismatches_when_symlink_escapes_worktree() {
    let env = TestEnv::new();
    let outside_file = env.task_dir.join("secret.txt");
    fs::write(&outside_file, "secret").unwrap();

    let symlink_path = env.worktree.join("escape.txt");
    std::os::unix::fs::symlink(&outside_file, &symlink_path).unwrap();

    let step = parse_first_step("[x] 1.1 Escaping symlink\n    Check: exists escape.txt\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert_eq!(result.check_line.as_deref(), Some("exists escape.txt"));
    assert!(result.reason.unwrap().contains("escape"));
}

#[tokio::test]
async fn check_exists_confirms_when_symlink_inside_worktree() {
    let env = TestEnv::new();
    let target_file = env.worktree.join("internal.txt");
    fs::write(&target_file, "ok").unwrap();

    let symlink_path = env.worktree.join("link.txt");
    std::os::unix::fs::symlink(&target_file, &symlink_path).unwrap();

    let step = parse_first_step("[x] 1.1 Internal symlink\n    Check: exists link.txt\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Confirmed);
    assert_eq!(result.check_line.as_deref(), Some("exists link.txt"));
    assert!(result.reason.is_none());
}

#[tokio::test]
async fn check_exists_mismatches_when_file_missing() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: exists missing.txt\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert_eq!(result.check_line.as_deref(), Some("exists missing.txt"));
    assert!(result.reason.unwrap().contains("missing.txt"));
}

#[tokio::test]
async fn check_reviewer_remains_declared() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: reviewer\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Declared);
    assert_eq!(result.check_line.as_deref(), Some("reviewer"));
    assert!(!result.gate_refused);
}

#[tokio::test]
async fn check_missing_remains_declared() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Declared);
    assert!(result.check_line.is_none());
}

#[tokio::test]
async fn check_refused_gate_command_remains_declared() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: agent gate --fast\n");
    let refuse = vec!["agent gate".to_owned(), "make gate".to_owned()];
    let ctx = env.context(120, &refuse);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Declared);
    assert_eq!(result.check_line.as_deref(), Some("agent gate --fast"));
    assert!(result.gate_refused);
}

#[tokio::test]
async fn check_command_remains_declared_in_part_a() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: echo passed check\n");
    let ctx = env.context(120, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Declared);
    assert_eq!(result.check_line.as_deref(), Some("echo passed check"));
    assert!(result.output_path.is_none());
}

#[test]
fn duration_millis_overflow_fails_with_duration_overflow() {
    let err = super::duration_millis(std::time::Duration::MAX).unwrap_err();
    assert!(matches!(err, super::CheckError::DurationOverflow { .. }));
}
