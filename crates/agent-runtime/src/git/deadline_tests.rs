//! Absolute capture deadlines are shared by every command of a snapshot.

use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::time::Duration;

use agent_core::config::{AbsolutePath, EnvName};
use tokio::time::{Instant, advance, pause};

use super::{CommitIdentity, Git, GitError, Revision};
use crate::process::CaptureError;

/// A scripted Git with a private PATH; never changes the test process's environment.
fn scripted(body: &str) -> (tempfile::TempDir, Git) {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("git");
    std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let git = Git::new(
        AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap(),
        CommitIdentity::new("Capture".to_owned(), "capture@example.com".to_owned(), None),
    )
    .with_env(EnvName::from_str("PATH").unwrap(), dir.path().to_str().unwrap());
    (dir, git)
}

#[tokio::test]
async fn an_expired_snapshot_deadline_refuses_to_spawn_git() {
    let (dir, git) = scripted("echo started > started\nprintf '%040d\\n' 1");
    let result = git.with_deadline(Instant::now()).rev_parse(&Revision::Head).await;
    assert!(
        matches!(
            result,
            Err(GitError::Run {
                source: CaptureError::Timeout { .. },
                ..
            })
        ),
        "{result:?}"
    );
    assert!(!dir.path().join("started").exists());
}

#[tokio::test]
async fn a_second_command_uses_the_snapshots_remaining_budget() {
    let (dir, git) = scripted("echo started >> started\nprintf '%040d\\n' 1");
    let git = git.with_deadline(Instant::now() + Duration::from_secs(2));
    git.rev_parse(&Revision::Head).await.unwrap();
    pause();
    advance(Duration::from_secs(2)).await;
    let running = tokio::spawn(async move { git.clone().rev_parse(&Revision::Head).await });
    for _ in 0..100_000 {
        if running.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(running.is_finished());
    let result = running.await.unwrap();
    assert!(
        matches!(
            result,
            Err(GitError::Run {
                source: CaptureError::Timeout { .. },
                ..
            })
        ),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("started")).unwrap(),
        "started\n"
    );
}

#[tokio::test]
async fn a_snapshot_deadline_kills_git_and_its_descendant_then_returns() {
    let (dir, git) = scripted("/bin/sleep 1000 &\nprintf '%s %s\\n' \"$$\" \"$!\" > ready\nwait");
    pause();
    let running = tokio::spawn(async move {
        git.with_deadline(Instant::now() + Duration::from_secs(2))
            .rev_parse(&Revision::Head)
            .await
    });
    // Keep a runnable future while waiting for the OS process: paused Tokio must not auto-advance.
    let ready = dir.path().join("ready");
    for _ in 0..100_000 {
        if std::fs::read_to_string(&ready).is_ok_and(|text| text.ends_with('\n')) {
            break;
        }
        tokio::task::yield_now().await;
    }
    let pids = std::fs::read_to_string(ready).unwrap();
    advance(Duration::from_secs(2)).await;
    for _ in 0..100_000 {
        if running.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let met_deadline = running.is_finished();
    if !met_deadline {
        // Clean up even on RED: the behavior under test does not yet terminate these processes.
        for pid in pids.split_whitespace() {
            let pid = rustix::process::Pid::from_raw(pid.parse().unwrap()).unwrap();
            match rustix::process::kill_process(pid, rustix::process::Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => panic!("cannot clean up test process: {error}"),
            }
        }
    }
    let result = running.await.unwrap();
    assert!(
        met_deadline,
        "snapshot exceeded its deadline without completing cleanup"
    );
    assert!(
        matches!(result, Err(GitError::Run { source: CaptureError::Timeout { limit }, .. }) if limit <= Duration::from_secs(2)),
        "{result:?}"
    );
    for pid in pids.split_whitespace() {
        let path = format!("/proc/{pid}/stat");
        let stopped = || match std::fs::read_to_string(&path) {
            Ok(stat) => stat.rsplit_once(") ").is_some_and(|(_, state)| state.starts_with('Z')),
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        for _ in 0..100_000 {
            if stopped() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(stopped(), "process {pid} survived the snapshot timeout");
    }
}
