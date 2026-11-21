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
    assert_eq!(result.reason.as_deref(), Some("refused gate command"));
    assert_eq!(result.tail, "");
    assert!(result.output_path.is_none());
    assert!(result.gate_refused);
}

#[tokio::test]
async fn execute_command_check_refuses_matching_prefix_before_spawn() {
    let env = TestEnv::new();
    let marker_file = env.worktree.join("refused-marker.txt");
    let cmd = format!("touch \"{}\"", marker_file.display());
    let step = parse_first_step(&format!("[x] 1.1 Refused\n    Check: {cmd}\n"));
    let refuse = vec!["touch".to_owned()];
    let ctx = env.context(60, &refuse);
    let result = super::execute_command_check(&ctx, &step, &cmd).await.unwrap();

    assert_eq!(result.state, JudgementState::Declared);
    assert!(result.gate_refused);
    assert_eq!(result.reason.as_deref(), Some("refused gate command"));
    assert!(!marker_file.exists(), "refused command must not be spawned");
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

#[tokio::test]
async fn check_command_confirms_on_success_and_logs_output() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: echo check-success-output\n");
    let ctx = env.context(60, &[]);
    let result = super::execute_command_check(&ctx, &step, "echo check-success-output")
        .await
        .unwrap();

    assert_eq!(result.state, JudgementState::Confirmed);
    assert_eq!(result.check_line.as_deref(), Some("echo check-success-output"));
    assert_eq!(
        result.output_path.as_deref(),
        Some("checks/run-20261121-010000-abcd/1.1.log")
    );
    let log_content = fs::read_to_string(env.task_dir.join("checks/run-20261121-010000-abcd/1.1.log")).unwrap();
    assert!(log_content.contains("check-success-output"));
}

#[tokio::test]
async fn check_command_mismatches_on_failure_with_tail() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: /bin/sh -c 'echo failure-detail; exit 2'\n");
    let ctx = env.context(60, &[]);
    let result = super::execute_command_check(&ctx, &step, "/bin/sh -c 'echo failure-detail; exit 2'")
        .await
        .unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(result.reason.as_deref().unwrap().contains("exit code 2"));
    assert_eq!(
        result.output_path.as_deref(),
        Some("checks/run-20261121-010000-abcd/1.1.log")
    );
    assert!(result.tail.contains("failure-detail"));
}

#[tokio::test]
async fn check_command_mismatches_on_timeout() {
    let env = TestEnv::new();
    let ready_file = env.worktree.join("ready.txt");
    let script_file = env.worktree.join("hang.sh");
    fs::write(
        &script_file,
        format!("#!/bin/sh\ntouch \"{}\"\nsleep 300\n", ready_file.display()),
    )
    .unwrap();
    let step = parse_first_step("[x] 1.1 First item\n    Check: sh hang.sh\n");
    let ctx = env.context(10, &[]);

    let check_fut = super::execute_command_check(&ctx, &step, "sh hang.sh");
    tokio::pin!(check_fut);

    let ready_fut = async {
        let start = std::time::Instant::now();
        while !ready_file.exists() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "timed out waiting for ready marker"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(11)).await;
        // Keep virtual time fixed while the OS delivers termination and reaps the process.
        let cleanup_started = std::time::Instant::now();
        loop {
            assert!(
                cleanup_started.elapsed() < std::time::Duration::from_secs(5),
                "cleanup did not finish"
            );
            tokio::task::yield_now().await;
        }
    };

    let result = tokio::select! {
        result = check_fut => result,
        () = ready_fut => panic!("readiness driver unexpectedly completed"),
    };
    let result = result.unwrap();
    assert_eq!(result.state, JudgementState::Mismatch);
    assert_eq!(result.reason.as_deref(), Some("ran past 10 seconds"));
}

#[tokio::test]
async fn check_command_disabled_when_seconds_zero() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: echo hello\n");
    let ctx = env.context(0, &[]);
    let result = super::execute_command_check(&ctx, &step, "echo hello").await.unwrap();

    assert_eq!(result.state, JudgementState::Declared);
    assert!(result.output_path.is_none());
}

#[tokio::test]
async fn check_command_receives_environment_and_has_no_stdin() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: test \"$MY_VAR\" = \"magic\" && ! read dummy\n");
    let envs = [("MY_VAR".to_owned(), "magic".to_owned())];
    let ctx = CheckContext {
        worktree: &env.worktree,
        task_dir: &env.task_dir,
        run: Some(&env.run),
        check_seconds: 60,
        check_refuse: &[],
        envs: &envs,
    };
    let result = super::execute_command_check(&ctx, &step, "test \"$MY_VAR\" = \"magic\" && ! read dummy")
        .await
        .unwrap();

    assert_eq!(result.state, JudgementState::Confirmed);
}

#[tokio::test]
async fn check_command_mismatches_on_command_not_found() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 Missing binary\n    Check: /nonexistent/binary\n");
    let ctx = env.context(60, &[]);
    let result = super::execute_command_check(&ctx, &step, "/nonexistent/binary")
        .await
        .unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(result.reason.as_deref().unwrap().contains("exit code 127"));
}

#[tokio::test]
async fn check_command_large_output_keeps_bounded_tail() {
    let env = TestEnv::new();
    let step = parse_first_step(
        "[x] 1.1 Big output\n    Check: python3 -c 'for i in range(1000): print(f\"line {i}\")'; exit 1\n",
    );
    let ctx = env.context(60, &[]);
    let cmd = "python3 -c 'for i in range(1000): print(f\"line {i}\")'; exit 1";
    let result = super::execute_command_check(&ctx, &step, cmd).await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    let lines: Vec<&str> = result.tail.lines().collect();
    assert_eq!(lines.len(), 40);
    assert_eq!(lines.last().copied(), Some("line 999"));
    assert_eq!(lines.first().copied(), Some("line 960"));

    let log_path = env.task_dir.join(result.output_path.as_ref().unwrap());
    let full_content = fs::read_to_string(&log_path).unwrap();
    assert_eq!(full_content.lines().count(), 1000);
}

#[tokio::test]
async fn check_command_invalid_utf8_is_lossy_decoded() {
    let env = TestEnv::new();
    let step =
        parse_first_step("[x] 1.1 Invalid utf8\n    Check: /bin/sh -c 'printf \"\\xff\\xfehello\\n\"; exit 1'\n");
    let ctx = env.context(60, &[]);
    let cmd = "/bin/sh -c 'printf \"\\xff\\xfehello\\n\"; exit 1'";
    let result = super::execute_command_check(&ctx, &step, cmd).await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(result.tail.contains("hello"));
}

#[tokio::test]
async fn check_command_kills_descendant_processes_in_group() {
    let env = TestEnv::new();
    let pid_file = env.worktree.join("child.pid");
    let pid_str = pid_file.to_str().unwrap();
    let cmd = format!("/bin/sh -c 'sleep 300 & echo $! > {pid_str}; exit 0'");
    let step = parse_first_step(&format!("[x] 1.1 Child proc\n    Check: {cmd}\n"));
    let ctx = env.context(60, &[]);
    let result = super::execute_command_check(&ctx, &step, &cmd).await.unwrap();
    assert_eq!(result.state, JudgementState::Confirmed);

    let pid_text = fs::read_to_string(&pid_file).unwrap();
    let pid: i32 = pid_text.trim().parse().unwrap();
    let proc_path = format!("/proc/{pid}");
    let path = std::path::Path::new(&proc_path);
    let start = std::time::Instant::now();
    while path.exists() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "timed out waiting for descendant process to be killed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!path.exists(), "descendant process must be killed");
}

#[tokio::test]
async fn check_command_guard_cleans_up_process_group_on_cancellation() {
    let env = TestEnv::new();
    let pid_file = env.worktree.join("cancel-child.pid");
    let pid_str = pid_file.to_str().unwrap();
    let cmd = format!("/bin/sh -c 'sleep 300 & echo $! > {pid_str}; sleep 60'");
    let step = parse_first_step(&format!("[x] 1.1 Cancel proc\n    Check: {cmd}\n"));
    let ctx = env.context(60, &[]);

    let mut check_fut = Box::pin(super::execute_command_check(&ctx, &step, &cmd));

    let pid: i32 = tokio::select! {
        _ = &mut check_fut => panic!("check must not finish before cancellation"),
        pid = async {
            let start = std::time::Instant::now();
            loop {
                if let Ok(text) = fs::read_to_string(&pid_file) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty()
                        && let Ok(parsed) = trimmed.parse::<i32>()
                    {
                        return parsed;
                    }
                }
                assert!(
                    start.elapsed() < std::time::Duration::from_secs(5),
                    "timed out waiting for child PID file"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        } => pid,
    };

    let proc_path = format!("/proc/{pid}");
    let path = std::path::Path::new(&proc_path);
    assert!(path.exists(), "descendant process must exist while running");

    drop(check_fut);

    let clean_start = std::time::Instant::now();
    while path.exists() {
        assert!(
            clean_start.elapsed() < std::time::Duration::from_secs(5),
            "timed out waiting for cancelled process to be cleaned up"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        !path.exists(),
        "guard drop must kill descendant process on cancellation"
    );
}

#[tokio::test]
async fn disabled_commands_with_check_seconds_zero_still_checks_exists() {
    let env = TestEnv::new();
    let file_path = env.worktree.join("docs/guide.md");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, "hello").unwrap();

    let step = parse_first_step("[x] 1.1 Exists item\n    Check: exists docs/guide.md\n");
    let ctx = env.context(0, &[]);
    let result = execute_step_check(&ctx, &step).await.unwrap();

    assert_eq!(result.state, JudgementState::Confirmed);
}

#[tokio::test]
async fn execute_command_check_mismatches_on_invalid_env_name() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: echo test\n");
    let envs = [("INVALID=NAME".to_owned(), "val".to_owned())];
    let ctx = CheckContext {
        worktree: &env.worktree,
        task_dir: &env.task_dir,
        run: Some(&env.run),
        check_seconds: 60,
        check_refuse: &[],
        envs: &envs,
    };
    let result = super::execute_command_check(&ctx, &step, "echo test").await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(
        result
            .reason
            .as_deref()
            .unwrap()
            .contains("invalid environment variable name")
    );
}

#[tokio::test]
async fn execute_command_check_mismatches_on_non_utf8_worktree() {
    use std::os::unix::ffi::OsStrExt;

    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 First item\n    Check: echo test\n");
    let non_utf8_path = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/test-\xff\xfe-path"));
    let ctx = CheckContext {
        worktree: non_utf8_path,
        task_dir: &env.task_dir,
        run: Some(&env.run),
        check_seconds: 60,
        check_refuse: &[],
        envs: &[],
    };
    let result = super::execute_command_check(&ctx, &step, "echo test").await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(
        result
            .reason
            .as_deref()
            .unwrap()
            .contains("worktree path is not valid UTF-8")
    );
}

#[tokio::test]
async fn preparation_failure_does_not_claim_an_output_file() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 Check\n    Check: true\n");
    let envs = [("INVALID=NAME".to_owned(), "value".to_owned())];
    let mut context = env.context(60, &[]);
    context.envs = &envs;
    let result = super::execute_command_check(&context, &step, "true").await.unwrap();
    assert_eq!(result.state, JudgementState::Mismatch);
    assert_eq!(result.output_path, None);
    assert!(
        !env.task_dir
            .join("checks")
            .join(env.run.as_str())
            .join("1.1.log")
            .exists()
    );
}

#[tokio::test]
async fn repeated_command_checks_replace_previous_output() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 Check\n    Check: printf current\n");
    let context = env.context(60, &[]);
    let first = super::execute_command_check(&context, &step, "printf stale; exit 1")
        .await
        .unwrap();
    assert_eq!(first.state, JudgementState::Mismatch);
    let second = super::execute_command_check(&context, &step, "printf current; exit 1")
        .await
        .unwrap();
    assert_eq!(second.state, JudgementState::Mismatch);
    assert_eq!(second.tail, "current");
    assert_eq!(
        fs::read_to_string(env.task_dir.join(second.output_path.unwrap())).unwrap(),
        "current"
    );
}

#[tokio::test]
async fn spawn_failure_reports_only_an_existing_output_file() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 Check\n    Check: true\n");
    let missing_worktree = env.worktree.join("missing");
    let mut context = env.context(60, &[]);
    context.worktree = &missing_worktree;
    let result = super::execute_command_check(&context, &step, "true").await.unwrap();
    assert_eq!(result.state, JudgementState::Mismatch);
    if let Some(path) = result.output_path {
        assert!(env.task_dir.join(path).is_file());
    }
}

#[tokio::test]
async fn missing_output_is_reported_in_the_mismatch_reason() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 Check\n    Check: true\n");
    let command = format!(
        "rm '{}/checks/{}/1.1.log'; exit 1",
        env.task_dir.display(),
        env.run.as_str()
    );
    let result = super::execute_command_check(&env.context(60, &[]), &step, &command)
        .await
        .unwrap();
    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(result.reason.unwrap().contains("cannot read check output"));
}

#[tokio::test]
async fn unreadable_output_is_reported_in_the_mismatch_reason() {
    let env = TestEnv::new();
    let step = parse_first_step("[x] 1.1 Check\n    Check: true\n");
    let command = format!(
        "rm '{0}/checks/{1}/1.1.log'; mkdir '{0}/checks/{1}/1.1.log'; exit 1",
        env.task_dir.display(),
        env.run.as_str()
    );
    let result = super::execute_command_check(&env.context(60, &[]), &step, &command)
        .await
        .unwrap();
    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(result.reason.unwrap().contains("cannot read check output"));
}

#[test]
fn failed_cleanup_retains_the_primary_and_cleanup_reasons() {
    for reason in ["ran past 1 seconds", "check wait failed: unavailable"] {
        let cleanup = Err(crate::process::ProcessError::StopTimeout {
            limit: std::time::Duration::from_secs(5),
        });
        assert_eq!(
            super::with_cleanup_result(reason.to_owned(), cleanup),
            format!("{reason}; process cleanup failed: the process did not end within 5s of SIGKILL")
        );
    }
    assert_eq!(
        super::with_cleanup_result("ran past 1 seconds".to_owned(), Ok(())),
        "ran past 1 seconds"
    );
}
