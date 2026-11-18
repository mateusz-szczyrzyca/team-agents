//! Tests for machine metrics and resource observation.

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use agent_core::config::{AbsolutePath, Secrets};
use agent_core::debug_summary::SummarySection;
use agent_core::diagnostic_assessment::{Anomaly, Assessment, Usage, assess};
use agent_core::ids::TaskId;
use agent_core::state::Timestamp;

use crate::clock::SystemClock;
use crate::debug_redaction::SecretsCut;
use crate::store::Home;

use super::*;

fn fixture() -> (tempfile::TempDir, Home, SecretsCut) {
    let dir = tempfile::tempdir().expect("tempdir");
    let home_path = dir.path().join("home");
    fs::create_dir_all(&home_path).expect("create home");
    let absolute = AbsolutePath::try_from(home_path.to_str().expect("utf8").to_owned()).expect("absolute");
    let home = Home::new(&absolute);
    let secrets = Secrets::default();
    let cut = SecretsCut::new(&secrets, []);
    (dir, home, cut)
}

/// Fixed collection instant used by injected machine observations.
fn test_now() -> Timestamp {
    "2026-11-18T12:00:00Z".parse::<Timestamp>().expect("valid timestamp")
}

fn write_proc_stat(proc_dir: &Path, pid: u32, ppid: u32) {
    let pid_dir = proc_dir.join(pid.to_string());
    fs::create_dir_all(&pid_dir).expect("create pid dir");
    // stat: pid (comm) state ppid ... field 22: starttime
    let stat = format!("{pid} (test_proc) S {ppid} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1000 0\n");
    fs::write(pid_dir.join("stat"), stat).expect("write stat");
}

fn write_proc_cmdline(proc_dir: &Path, pid: u32, args: &[&str]) {
    let pid_dir = proc_dir.join(pid.to_string());
    let mut data = Vec::new();
    for arg in args {
        data.extend_from_slice(arg.as_bytes());
        data.push(0);
    }
    fs::write(pid_dir.join("cmdline"), data).expect("write cmdline");
}

#[tokio::test]
async fn reads_loadavg_meminfo_filesystems_and_daemon_descendants() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create procfs");

    fs::write(procfs.join("loadavg"), "0.15 0.25 0.35 1/200 1234\n").expect("write loadavg");
    fs::write(
        procfs.join("meminfo"),
        "MemTotal:       16384000 kB\nMemAvailable:    8192000 kB\nSwapTotal:       4194304 kB\nSwapFree:        2097152 kB\n",
    )
    .expect("write meminfo");

    let daemon_pid = 100;
    // 100: daemon
    write_proc_stat(&procfs, 100, 1);
    write_proc_cmdline(&procfs, 100, &["agent", "daemon"]);

    // 200: child of daemon
    write_proc_stat(&procfs, 200, 100);
    write_proc_cmdline(&procfs, 200, &["agent", "worker", "--task", "task-1"]);

    // 300: grandchild of daemon (child of 200)
    write_proc_stat(&procfs, 300, 200);
    write_proc_cmdline(&procfs, 300, &["cargo", "test"]);

    // 400: not a descendant of daemon (child of 1)
    write_proc_stat(&procfs, 400, 1);
    write_proc_cmdline(&procfs, 400, &["other", "daemon"]);

    // Tasks and worktrees directories
    let tasks_dir = home.root().join("tasks");
    fs::create_dir_all(tasks_dir.join("task-1")).expect("create task dir");
    fs::write(tasks_dir.join("task-1/meta.env"), "KEY=VALUE\n").expect("write task meta");

    let worktrees_dir = home.root().join("worktrees");
    fs::create_dir_all(worktrees_dir.join("wt-1")).expect("create wt dir");
    fs::write(worktrees_dir.join("wt-1/file.txt"), "hello world\n").expect("write wt file");

    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let obs = observe(&home, &procfs, &temp_dir, Some(daemon_pid), &cut, test_now()).await;

    assert_eq!(obs.summary.load_avg, Some((0.15, 0.25, 0.35)));
    assert_eq!(obs.summary.memory_total_bytes, Some(16_384_000 * 1024));
    assert_eq!(obs.summary.memory_available_bytes, Some(8_192_000 * 1024));
    assert!(obs.summary.home_fs.is_some());
    assert!(obs.summary.temp_fs.is_some());
    assert_eq!(obs.summary.daemon_descendant_pids, vec![200, 300]);
    assert_eq!(obs.summary.tasks_dir_bytes, Some(10));
    assert_eq!(obs.summary.worktrees_dir_bytes, Some(12));

    let artifact = String::from_utf8(obs.artifact_bytes).expect("utf8 artifact");
    assert!(artifact.contains("Load average: 0.15, 0.25, 0.35"), "{artifact}");
    assert!(artifact.contains("Memory:"), "{artifact}");
    assert!(artifact.contains("Swap:"), "{artifact}");
    assert!(artifact.contains("PID 200"), "{artifact}");
    assert!(artifact.contains("PID 300"), "{artifact}");
    assert!(!artifact.contains("PID 400"), "{artifact}");
}

#[tokio::test]
async fn unreadable_machine_sources_record_failures_under_machine_section() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("empty-proc");
    fs::create_dir_all(&procfs).expect("create empty proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let obs = observe(&home, &procfs, &temp_dir, Some(100), &cut, test_now()).await;

    assert_eq!(obs.summary.load_avg, None);
    assert_eq!(obs.summary.memory_total_bytes, None);
    assert_eq!(obs.summary.memory_available_bytes, None);
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "load average")
    );
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "memory")
    );
}

#[tokio::test]
async fn home_and_temp_on_same_filesystem_recorded_once_in_assessment() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    // Both home and temp_dir are inside `dir`, so they share the same filesystem
    let obs = observe(&home, &procfs, &temp_dir, None, &cut, test_now()).await;
    assert_eq!(obs.filesystems.len(), 1, "same filesystem must be recorded once");
    assert_eq!(obs.filesystems[0].path, home.root());
}

#[test]
fn filesystem_90_percent_full_by_space_or_inodes_triggers_anomaly() {
    let path = PathBuf::from("/test/fs");
    let full_space = Filesystem {
        path: path.clone(),
        space: Some(Usage {
            used: 900,
            total: NonZeroU64::new(1000).expect("non-zero"),
        }),
        inodes: Some(Usage {
            used: 100,
            total: NonZeroU64::new(1000).expect("non-zero"),
        }),
    };

    let now = Timestamp::now(&SystemClock).expect("now");
    let lead: TaskId = "task-20261118-120000-lead".parse().expect("task id");
    let assessment = Assessment {
        now,
        stale_after: std::time::Duration::from_secs(3600),
        daemon: None,
        command_version: None,
        lead,
        tasks: vec![],
        items: vec![],
        lead_git: None,
        branches: vec![],
        filesystems: vec![full_space],
        unanswered: vec![],
        unreadable: vec![],
    };

    let anomalies = assess(&assessment);
    assert!(
        anomalies.contains(&Anomaly::FullFilesystem(path.clone())),
        "90% space usage must trigger FullFilesystem anomaly"
    );

    // Below 90%
    let ok_space = Filesystem {
        path: path.clone(),
        space: Some(Usage {
            used: 899,
            total: NonZeroU64::new(1000).expect("non-zero"),
        }),
        inodes: Some(Usage {
            used: 100,
            total: NonZeroU64::new(1000).expect("non-zero"),
        }),
    };
    let mut ok_assessment = assessment.clone();
    ok_assessment.filesystems = vec![ok_space];
    let ok_anomalies = assess(&ok_assessment);
    assert!(
        !ok_anomalies.contains(&Anomaly::FullFilesystem(path.clone())),
        "89.9% space usage must not trigger FullFilesystem anomaly"
    );

    // 90% inodes
    let full_inodes = Filesystem {
        path: path.clone(),
        space: Some(Usage {
            used: 100,
            total: NonZeroU64::new(1000).expect("non-zero"),
        }),
        inodes: Some(Usage {
            used: 900,
            total: NonZeroU64::new(1000).expect("non-zero"),
        }),
    };
    let mut inodes_assessment = assessment.clone();
    inodes_assessment.filesystems = vec![full_inodes];
    let inodes_anomalies = assess(&inodes_assessment);
    assert!(
        inodes_anomalies.contains(&Anomaly::FullFilesystem(path.clone())),
        "90% inode usage must trigger FullFilesystem anomaly"
    );
}

#[tokio::test]
async fn daemon_descendants_collects_only_descendants_and_redacts_secrets_in_cmdline() {
    let (dir, home, _) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create procfs");

    let secrets_content = "github_token = \"super_secret_token_12345\"\n";
    let secrets = Secrets::parse(secrets_content, 0o600).expect("secrets");
    let cut = SecretsCut::new(&secrets, []);

    let daemon_pid = 50;
    write_proc_stat(&procfs, 50, 1);
    write_proc_cmdline(&procfs, 50, &["agent", "daemon"]);

    write_proc_stat(&procfs, 51, 50);
    write_proc_cmdline(
        &procfs,
        51,
        &["agent", "worker", "--secret", "super_secret_token_12345"],
    );

    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let obs = observe(&home, &procfs, &temp_dir, Some(daemon_pid), &cut, test_now()).await;
    assert_eq!(obs.summary.daemon_descendant_pids, vec![51]);

    let artifact = String::from_utf8(obs.artifact_bytes).expect("utf8 artifact");
    assert!(
        !artifact.contains("super_secret_token_12345"),
        "secret must be redacted in machine artifact: {artifact}"
    );
    assert!(
        artifact.contains("<cut>"),
        "redacted marker must be present in machine artifact: {artifact}"
    );
}

#[tokio::test]
async fn tasks_and_worktrees_directory_sizes_measured() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let tasks = home.root().join("tasks");
    fs::create_dir_all(tasks.join("t1/sub")).expect("create sub");
    fs::write(tasks.join("t1/a.txt"), "12345").expect("write a");
    fs::write(tasks.join("t1/sub/b.txt"), "6789012345").expect("write b");

    let worktrees = home.root().join("worktrees");
    fs::create_dir_all(worktrees.join("w1")).expect("create w1");
    fs::write(worktrees.join("w1/c.txt"), "hello").expect("write c");

    let obs = observe(&home, &procfs, &temp_dir, None, &cut, test_now()).await;
    assert_eq!(obs.summary.tasks_dir_bytes, Some(15));
    assert_eq!(obs.summary.worktrees_dir_bytes, Some(5));
}
