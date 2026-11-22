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

#[tokio::test]
async fn output_creation_failure_mismatches_without_running_command() {
    let env = TestEnv::new();
    let command = "touch executed";
    let step = parse_first_step("[x] 1.1 Check\n    Check: touch executed\n");
    let log_path = env.task_dir.join("checks").join(env.run.as_str()).join("1.1.log");
    fs::create_dir_all(&log_path).unwrap();

    let result = super::execute_command_check(&env.context(60, &[]), &step, command)
        .await
        .unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(result.reason.unwrap().starts_with("cannot create check output: "));
    assert_eq!(result.output_path, None);
    assert!(!env.worktree.join("executed").exists());
}

/// The path of the record of the check that runs in `task_dir`.
fn running_record(task_dir: &std::path::Path) -> std::path::PathBuf {
    task_dir.join("checks").join("running")
}

/// Reads the kernel start time (field 22 of `/proc/<pid>/stat`) of `pid`.
fn start_ticks_of(pid: u32) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("read proc stat");
    let (_, after_comm) = stat.rsplit_once(')').expect("comm in parens");
    after_comm
        .split_whitespace()
        .nth(19)
        .expect("starttime field")
        .parse()
        .expect("starttime u64")
}

#[tokio::test]
async fn check_command_records_its_process_while_it_runs_and_clears_the_record_when_it_ends() {
    let env = TestEnv::new();
    let command = "sleep 30";
    let step = parse_first_step("[x] 1.1 First item\n    Check: sleep 30\n");
    let record = running_record(&env.task_dir);
    let refuse: Vec<String> = Vec::new();
    let ctx = CheckContext {
        worktree: &env.worktree,
        task_dir: &env.task_dir,
        run: Some(&env.run),
        check_seconds: 1,
        check_refuse: &refuse,
        envs: &[],
    };

    let running = super::execute_command_check(&ctx, &step, command);
    tokio::pin!(running);
    let mut seen = None;
    let result = loop {
        tokio::select! {
            result = &mut running => break result.unwrap(),
            () = tokio::time::sleep(std::time::Duration::from_millis(10)), if seen.is_none() => {
                seen = fs::read_to_string(&record).ok().filter(|text| !text.trim().is_empty());
            }
        }
    };

    let record_text = seen.expect("the record of the process is written while the check runs");
    let (pid, ticks) = record_text
        .trim()
        .split_once(' ')
        .expect("the record holds a process ID and start time");
    assert!(
        pid.parse::<u32>().is_ok() && ticks.parse::<u64>().is_ok(),
        "the record holds a process ID and start time: {record_text:?}"
    );
    assert_eq!(result.state, JudgementState::Mismatch);
    assert_eq!(result.reason.as_deref(), Some("ran past 1 seconds"));
    assert!(!record.exists(), "the record is cleared when the check has ended");
}

#[test]
fn recover_checks_ends_the_group_of_a_check_left_by_a_dead_daemon() {
    use std::os::unix::process::CommandExt;

    let env = TestEnv::new();
    let record = running_record(&env.task_dir);
    fs::create_dir_all(record.parent().expect("checks dir")).expect("checks dir");
    let mut child = std::process::Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let ticks = start_ticks_of(child.id());
    fs::write(&record, format!("{} {ticks}\n", child.id())).expect("write record");

    let ended = super::recover_checks(&env.task_dir).expect("recovery");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    if status.is_none() {
        child.kill().expect("cleanup kill");
        child.wait().expect("cleanup wait");
    }
    assert!(status.is_some(), "the group of the left-over check is ended");
    assert!(ended.is_some(), "the recovery reports the group it ended");
    assert!(!record.exists(), "the record is removed by the recovery");
}

#[test]
fn recover_checks_without_a_record_does_nothing() {
    let env = TestEnv::new();

    let ended = super::recover_checks(&env.task_dir).expect("recovery");

    assert_eq!(ended, None);
}

#[tokio::test]
async fn check_command_mismatches_without_running_when_its_record_cannot_be_written() {
    let env = TestEnv::new();
    // A directory where the record file belongs makes the record impossible to write.
    fs::create_dir_all(running_record(&env.task_dir)).expect("record blocker");
    let step = parse_first_step("[x] 1.1 First item\n    Check: echo never\n");
    let ctx = env.context(60, &[]);

    let result = super::execute_command_check(&ctx, &step, "echo never").await.unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(
        result
            .reason
            .as_deref()
            .unwrap()
            .starts_with("cannot record the running check"),
        "reason: {:?}",
        result.reason
    );
}

#[test]
fn recover_checks_leaves_an_unrelated_process_group_alone_when_start_time_differs() {
    use std::os::unix::process::CommandExt;

    let env = TestEnv::new();
    let record = running_record(&env.task_dir);
    fs::create_dir_all(record.parent().expect("checks dir")).expect("checks dir");
    let mut child = std::process::Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let ticks = start_ticks_of(child.id());
    fs::write(&record, format!("{} {}\n", child.id(), ticks.saturating_add(1))).expect("write mismatched record");

    let ended = super::recover_checks(&env.task_dir).expect("recovery ignores reused PID");
    let still_alive = child.try_wait().expect("try_wait").is_none();
    child.kill().expect("cleanup kill");
    child.wait().expect("cleanup wait");

    assert_eq!(ended, None);
    assert!(
        still_alive,
        "unrelated process with different start time is not signalled"
    );
    assert!(!record.exists(), "stale record is removed");
}

#[tokio::test]
async fn check_command_does_not_spawn_process_when_record_cannot_be_written() {
    let env = TestEnv::new();
    fs::create_dir_all(running_record(&env.task_dir)).expect("record blocker");
    let step = parse_first_step("[x] 1.1 First item\n    Check: touch spawned.txt\n");
    let ctx = env.context(60, &[]);

    let result = super::execute_command_check(&ctx, &step, "touch spawned.txt")
        .await
        .unwrap();

    assert_eq!(result.state, JudgementState::Mismatch);
    assert!(
        result
            .reason
            .as_deref()
            .unwrap()
            .starts_with("cannot record the running check"),
        "reason: {:?}",
        result.reason
    );
    assert!(
        !env.worktree.join("spawned.txt").exists(),
        "command must not run before its record is writable"
    );
}

#[test]
fn recover_checks_ends_surviving_descendants_after_group_leader_exits_and_is_reaped() {
    use std::os::unix::process::CommandExt;

    let dir = tempfile::tempdir().unwrap();
    let checks_dir = dir.path().join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    let record_path = checks_dir.join("running");
    let child_pid_path = dir.path().join("child.pid");

    let mut leader = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("rec=\"$1\"; cpid=\"$2\"; read -r s < \"/proc/$$/stat\" && set -- ${s#*)} && printf '%s %s\\n' \"$$\" \"${20}\" > \"$rec\"; sleep 30 >/dev/null 2>&1 & printf '%s\\n' \"$!\" > \"$cpid\"")
        .arg("sh")
        .arg(&record_path)
        .arg(&child_pid_path)
        .process_group(0)
        .spawn()
        .expect("spawn group leader");
    let leader_pid = leader.id();
    let status = leader.wait().expect("wait and reap group leader");
    assert!(status.success(), "leader exited cleanly: {status:?}");

    let child_pid: u32 = std::fs::read_to_string(&child_pid_path)
        .expect("read child.pid")
        .trim()
        .parse()
        .expect("parse child pid");
    let child_proc_stat = format!("/proc/{child_pid}/stat");
    assert!(
        std::path::Path::new(&child_proc_stat).exists(),
        "descendant process {child_pid} must still be alive after leader exit"
    );

    let recovered = super::recover_checks(dir.path()).expect("recover checks after leader exit");
    if recovered.is_none()
        && let Ok(raw) = i32::try_from(leader_pid)
        && let Some(group) = rustix::process::Pid::from_raw(raw)
    {
        drop(crate::process::signal_group(group, crate::process::GroupSignal::Kill));
    }
    assert_eq!(
        recovered,
        Some(std::num::NonZeroU32::new(leader_pid).unwrap()),
        "recover_checks must still identify and signal the process group after the leader exited and was reaped"
    );
    assert!(!record_path.exists(), "running record must be removed after recovery");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::path::Path::new(&child_proc_stat).exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let alive = std::fs::read_to_string(&child_proc_stat).is_ok_and(|s| !s.contains(") Z "));
    assert!(
        !alive,
        "surviving descendant process {child_pid} must be killed by recover_checks"
    );
}

#[tokio::test]
async fn execute_command_check_keeps_running_record_non_empty_and_reports_remove_failure() {
    let env = TestEnv::new();
    let missing_dir = env.task_dir.join("not-a-dir-file");
    std::fs::write(&missing_dir, b"file").unwrap();
    let unremovable = missing_dir.join("running");
    let combined = super::remove_running_record(&unremovable, "primary error".to_owned()).await;
    assert!(
        combined.contains("primary error; cannot remove the record of the running check"),
        "remove_running_record must append non-NotFound removal errors: {combined}"
    );
}

#[tokio::test]
async fn spawning_daemon_death_before_record_write_prevents_child_command_from_running() {
    let env = TestEnv::new();
    let checks_dir = env.task_dir.join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    let record = checks_dir.join("running");
    std::fs::write(&record, b"").unwrap();
    let log_path = checks_dir.join("dead-daemon.log");
    std::fs::write(&log_path, b"").unwrap();

    let ctx = env.context(10, &[]);
    let spec = super::prepare_process_spec_for_daemon(
        &ctx,
        "touch ran_after_daemon_death.txt",
        &record,
        log_path,
        std::process::id(),
        u64::MAX,
    )
    .expect("prepare process spec for dead daemon");
    let supervisor = crate::process::Supervisor::new(std::num::NonZeroUsize::MIN);
    let mut process = supervisor.spawn(&spec).await.expect("spawn child shell");
    let _exit = process.wait().await.expect("wait for child shell");

    assert!(
        !env.worktree.join("ran_after_daemon_death.txt").exists(),
        "child shell must not execute the check command when the spawning daemon died before the record write"
    );
}

#[tokio::test]
async fn replace_running_record_and_proc_scan_propagate_errors() {
    let env = TestEnv::new();
    let not_a_dir = env.task_dir.join("regular-file");
    std::fs::write(&not_a_dir, b"file").unwrap();
    let bad_temp = not_a_dir.join("running.daemon");
    let target_dir = env.task_dir.join("target-dir");
    std::fs::create_dir_all(&target_dir).unwrap();

    let rename_err = super::replace_running_record(&bad_temp, &target_dir)
        .await
        .expect_err("rename from invalid path must fail");
    assert!(
        rename_err.to_string().contains("cannot remove temporary record"),
        "replace_running_record must retain the temp-file removal error alongside the rename error: {rename_err}"
    );

    let fake_proc = env.task_dir.join("fake-proc");
    let bad_pid_dir = fake_proc.join("4242");
    std::fs::create_dir_all(&bad_pid_dir).unwrap();
    std::fs::write(bad_pid_dir.join("stat"), "corrupt-stat-without-parens").unwrap();

    let scan_err = super::process_group_has_member_since_in(&fake_proc, 4242, 100)
        .expect_err("process_group_has_member_since_in must propagate unexpected /proc/<pid>/stat errors");
    assert_eq!(scan_err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn prepare_process_spec_in_returns_daemon_start_time_error_when_proc_stat_is_missing_or_corrupt() {
    let env = TestEnv::new();
    let checks_dir = env.task_dir.join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    let record = checks_dir.join("running");
    let log_path = checks_dir.join("prep.log");
    let ctx = env.context(10, &[]);

    let empty_proc = env.task_dir.join("empty-proc");
    std::fs::create_dir_all(&empty_proc).unwrap();
    let missing_err = super::prepare_process_spec_in(&empty_proc, &ctx, "exit 0", &record, log_path.clone(), 4242)
        .expect_err("missing daemon /proc/<pid>/stat must fail process spec preparation");
    assert!(
        matches!(
            &missing_err,
            super::CommandPreparationError::DaemonStartTime { pid: 4242, source, .. }
                if source.kind() == std::io::ErrorKind::NotFound
        ),
        "expected DaemonStartTime with NotFound, got {missing_err:?}"
    );

    let corrupt_proc = env.task_dir.join("corrupt-proc");
    let pid_dir = corrupt_proc.join("4242");
    std::fs::create_dir_all(&pid_dir).unwrap();
    std::fs::write(pid_dir.join("stat"), "corrupt-daemon-stat").unwrap();
    let corrupt_err = super::prepare_process_spec_in(&corrupt_proc, &ctx, "exit 0", &record, log_path, 4242)
        .expect_err("corrupt daemon /proc/<pid>/stat must fail process spec preparation");
    assert!(
        matches!(
            &corrupt_err,
            super::CommandPreparationError::DaemonStartTime { pid: 4242, source, .. }
                if source.kind() == std::io::ErrorKind::InvalidData
        ),
        "expected DaemonStartTime with InvalidData, got {corrupt_err:?}"
    );
}

#[tokio::test]
async fn spawning_daemon_death_during_record_rename_removes_record_and_prevents_command() {
    let env = TestEnv::new();
    let checks_dir = env.task_dir.join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    let record = checks_dir.join("running");
    std::fs::write(&record, b"").unwrap();
    let log_path = checks_dir.join("post-rename-dead-daemon.log");
    std::fs::write(&log_path, b"").unwrap();

    let mut stand_in = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("exec sleep 30")
        .spawn()
        .expect("spawn stand-in daemon");
    let stand_in_pid = stand_in.id();
    let stand_in_ticks = super::process_start_ticks(stand_in_pid)
        .expect("read stand-in daemon stat")
        .expect("stand-in daemon must exist");

    let fake_bin = env.task_dir.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).unwrap();
    let mv_wrapper = fake_bin.join("mv");
    std::fs::write(
        &mv_wrapper,
        format!(
            "#!/bin/sh\n/bin/mv \"$@\"\nstatus=$?\nkill -s KILL {stand_in_pid} 2>/dev/null || true\nwhile read -r ds < \"/proc/{stand_in_pid}/stat\" 2>/dev/null; do\n  set -- ${{ds#*)}}\n  [ \"$1\" = \"Z\" ] && break\ndone\nexit $status\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mv_wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())
    );
    let envs = vec![("PATH".to_owned(), path_env)];
    let ctx = super::CheckContext {
        worktree: &env.worktree,
        task_dir: &env.task_dir,
        run: None,
        check_seconds: 10,
        check_refuse: &[],
        envs: &envs,
    };
    let spec = super::prepare_process_spec_for_daemon(
        &ctx,
        "touch ran_after_post_rename_death.txt",
        &record,
        log_path,
        stand_in_pid,
        stand_in_ticks,
    )
    .expect("prepare process spec for stand-in daemon");
    let supervisor = crate::process::Supervisor::new(std::num::NonZeroUsize::MIN);
    let mut process = supervisor.spawn(&spec).await.expect("spawn child shell");
    let exit = process.wait().await.expect("wait for child shell");
    drop(stand_in.kill());
    drop(stand_in.wait());

    assert_eq!(
        exit,
        crate::process::ExitReport::Exited { code: 125 },
        "child shell must exit 125 when the spawning daemon dies during the record rename"
    );
    assert!(
        !env.worktree.join("ran_after_post_rename_death.txt").exists(),
        "child shell must not execute the check command when the second daemon-liveness check fails"
    );
    assert!(
        !record.exists(),
        "child shell must remove checks/running when the second daemon-liveness check fails"
    );
}
