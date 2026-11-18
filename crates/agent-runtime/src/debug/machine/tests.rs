//! Tests for machine metrics and resource observation.

use std::fs;
use std::num::NonZeroU64;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use agent_core::config::{AbsolutePath, Secrets};
use agent_core::debug_summary::SummarySection;
use agent_core::diagnostic_assessment::{Anomaly, Assessment, Usage, assess};
use agent_core::ids::TaskId;
use agent_core::state::Timestamp;

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

fn test_now() -> Timestamp {
    "2026-11-18T12:00:00Z".parse::<Timestamp>().expect("valid timestamp")
}

fn write_proc_stat(proc_dir: &Path, pid: u32, parent_pid: u32) {
    let pid_dir = proc_dir.join(pid.to_string());
    fs::create_dir_all(&pid_dir).expect("create pid dir");
    // stat: pid (comm) state ppid ... field 22: starttime
    let stat = format!("{pid} (test_proc) S {parent_pid} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1000 0\n");
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

    let now = "2026-11-18T12:00:00Z".parse::<Timestamp>().expect("valid timestamp");
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

#[tokio::test]
async fn observed_filesystem_90_percent_space_triggers_anomaly_and_below_does_not() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let home_path = home.root().to_path_buf();
    let statvfs_mock = move |path: &Path| -> Result<StatvfsStats, rustix::io::Errno> {
        if path == home_path {
            Ok(StatvfsStats {
                fragment_size: 4096,
                block_size: 4096,
                total_blocks: 100,
                available_blocks: 10,
                total_inodes: 100,
                available_inodes: 80,
            })
        } else {
            Ok(StatvfsStats {
                fragment_size: 4096,
                block_size: 4096,
                total_blocks: 100,
                available_blocks: 50,
                total_inodes: 100,
                available_inodes: 80,
            })
        }
    };

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: "2026-11-18T12:00:00Z".parse::<Timestamp>().unwrap(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, statvfs_mock).await;
    assert_eq!(obs.summary.home_fs.unwrap().space_percent, Some(90));
    assert_eq!(obs.summary.temp_fs.unwrap().space_percent, Some(50));

    let assessment = Assessment {
        now: "2026-11-18T12:00:00Z".parse::<Timestamp>().unwrap(),
        stale_after: std::time::Duration::from_secs(3600),
        daemon: None,
        command_version: None,
        lead: "task-20261118-120000-lead".parse().unwrap(),
        tasks: vec![],
        items: vec![],
        lead_git: None,
        branches: vec![],
        filesystems: obs.filesystems,
        unanswered: vec![],
        unreadable: vec![],
    };

    let anomalies = assess(&assessment);
    assert!(
        anomalies.contains(&Anomaly::FullFilesystem(home.root().to_path_buf())),
        "90% space usage must trigger FullFilesystem anomaly"
    );
    assert!(
        !anomalies.contains(&Anomaly::FullFilesystem(temp_dir.clone())),
        "50% space usage must not trigger FullFilesystem anomaly"
    );
}

#[tokio::test]
async fn observed_filesystem_90_percent_inodes_triggers_anomaly() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let statvfs_mock = move |_path: &Path| -> Result<StatvfsStats, rustix::io::Errno> {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 80,
            total_inodes: 100,
            available_inodes: 10,
        })
    };

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: "2026-11-18T12:00:00Z".parse::<Timestamp>().unwrap(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, statvfs_mock).await;
    assert_eq!(obs.summary.home_fs.unwrap().inode_percent, Some(90));

    let assessment = Assessment {
        now: "2026-11-18T12:00:00Z".parse::<Timestamp>().unwrap(),
        stale_after: std::time::Duration::from_secs(3600),
        daemon: None,
        command_version: None,
        lead: "task-20261118-120000-lead".parse().unwrap(),
        tasks: vec![],
        items: vec![],
        lead_git: None,
        branches: vec![],
        filesystems: obs.filesystems,
        unanswered: vec![],
        unreadable: vec![],
    };

    let anomalies = assess(&assessment);
    assert!(
        anomalies.contains(&Anomaly::FullFilesystem(home.root().to_path_buf())),
        "90% inode usage must trigger FullFilesystem anomaly"
    );
}

#[tokio::test]
async fn observed_descendants_compute_age_from_proc_stat_btime() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    fs::write(procfs.join("stat"), "cpu 0 0 0 0\nbtime 1700000000\n").expect("write stat");

    let daemon_pid = 10;
    write_proc_stat(&procfs, 10, 1);
    write_proc_cmdline(&procfs, 10, &["agent", "daemon"]);

    let pid_dir = procfs.join("20");
    fs::create_dir_all(&pid_dir).expect("create pid dir");
    let stat = "20 (worker) S 10 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1000 0\n";
    fs::write(pid_dir.join("stat"), stat).expect("write stat");
    write_proc_cmdline(&procfs, 20, &["worker", "run"]);

    let collected_at = Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(1_700_000_035).unwrap()).unwrap();

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: Some(daemon_pid),
        cut: &cut,
        collected_at,
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, |_| {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 50,
            total_inodes: 100,
            available_inodes: 50,
        })
    })
    .await;

    assert_eq!(obs.summary.daemon_descendants.len(), 1);
    assert_eq!(obs.summary.daemon_descendants[0].pid, 20);
    assert_eq!(obs.summary.daemon_descendants[0].age_seconds, Some(25));
    assert_eq!(obs.summary.daemon_descendants[0].cmdline, vec!["worker", "run"]);
}

#[tokio::test]
async fn missing_meminfo_memavailable_field_records_unreadable_failure() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    fs::write(
        procfs.join("meminfo"),
        "MemTotal:       16384000 kB\nMemFree:        8192000 kB\n",
    )
    .expect("write meminfo");

    let obs = observe(&home, &procfs, &temp_dir, None, &cut, test_now()).await;
    assert_eq!(obs.summary.memory_available_bytes, None);
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "memory" && f.reason.contains("MemAvailable")),
        "missing MemAvailable must record unreadable failure without substituting MemFree: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn descendant_cycle_terminates_without_infinite_loop() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let daemon_pid = 100;
    write_proc_stat(&procfs, 100, 1);
    write_proc_cmdline(&procfs, 100, &["daemon"]);

    write_proc_stat(&procfs, 200, 100);
    write_proc_cmdline(&procfs, 200, &["proc200"]);

    write_proc_stat(&procfs, 201, 200);
    write_proc_cmdline(&procfs, 201, &["proc201"]);

    write_proc_stat(&procfs, 202, 201);
    write_proc_cmdline(&procfs, 202, &["proc202"]);

    let obs = observe(&home, &procfs, &temp_dir, Some(daemon_pid), &cut, test_now()).await;
    let pids: Vec<u32> = obs.summary.daemon_descendants.iter().map(|d| d.pid).collect();
    assert_eq!(pids, vec![200, 201, 202]);
}

#[tokio::test]
async fn malformed_meminfo_numeric_field_records_error() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    fs::write(
        procfs.join("meminfo"),
        "MemTotal:       invalid_kb\nMemAvailable:   1000 kB\n",
    )
    .expect("write meminfo");

    let obs = observe(&home, &procfs, &temp_dir, None, &cut, test_now()).await;
    assert!(
        obs.unreadable.iter().any(|f| f.section == SummarySection::Machine
            && f.part == "memory"
            && f.reason.contains("invalid numeric value")),
        "malformed numeric field must record error: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn meminfo_checked_mul_overflow_records_error() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    fs::write(
        procfs.join("meminfo"),
        "MemTotal:       18446744073709551615 kB\nMemAvailable:   1000 kB\n",
    )
    .expect("write meminfo");

    let obs = observe(&home, &procfs, &temp_dir, None, &cut, test_now()).await;
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "memory" && f.reason.contains("overflow")),
        "multiplication overflow must record error: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn unavailable_swap_records_unreadable_reason() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    fs::write(
        procfs.join("meminfo"),
        "MemTotal:       10000 kB\nMemAvailable:   5000 kB\n",
    )
    .expect("write meminfo");

    let obs = observe(&home, &procfs, &temp_dir, None, &cut, test_now()).await;
    assert_eq!(obs.summary.swap_total_bytes, None);
    assert_eq!(obs.summary.swap_free_bytes, None);
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "swap"),
        "unavailable swap must record unreadable failure: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn unavailable_clock_ticks_or_btime_reports_age_unavailable() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let daemon_pid = 100;
    write_proc_stat(&procfs, 100, 1);
    write_proc_cmdline(&procfs, 100, &["daemon"]);
    write_proc_stat(&procfs, 200, 100);
    write_proc_cmdline(&procfs, 200, &["worker"]);

    // No /proc/stat file created, so btime is missing
    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: Some(daemon_pid),
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: None,
    };

    let obs = observe_with_statvfs(params, |_| {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 50,
            total_inodes: 100,
            available_inodes: 50,
        })
    })
    .await;

    assert_eq!(obs.summary.daemon_descendants.len(), 1);
    assert_eq!(obs.summary.daemon_descendants[0].pid, 200);
    assert_eq!(obs.summary.daemon_descendants[0].age_seconds, None);
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "process age"),
        "missing btime/ticks must report process age unavailable: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn process_disappeared_race_ignored_while_malformed_stat_recorded() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    fs::write(procfs.join("stat"), "btime 1700000000\n").expect("write stat");

    let daemon_pid = 100;
    write_proc_stat(&procfs, 100, 1);
    write_proc_cmdline(&procfs, 100, &["daemon"]);

    // Process 200: directory exists, but stat file does NOT exist (disappeared race)
    fs::create_dir_all(procfs.join("200")).expect("create 200");

    // Process 300: directory exists, but stat file is malformed
    fs::create_dir_all(procfs.join("300")).expect("create 300");
    fs::write(procfs.join("300/stat"), "malformed stat content\n").expect("write 300 stat");
    write_proc_cmdline(&procfs, 300, &["proc300"]);

    // Process 400: non-numeric directory (e.g. sys, net)
    fs::create_dir_all(procfs.join("sys")).expect("create sys");

    let obs = observe(&home, &procfs, &temp_dir, Some(daemon_pid), &cut, test_now()).await;

    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part.contains("process 300")),
        "malformed stat must be recorded as unreadable: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn descendant_collection_failure_does_not_print_none_in_artifact() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("unreadable-proc");
    // Make procfs a file instead of directory to cause read_dir failure
    fs::write(&procfs, "not a directory").expect("write file");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let obs = observe(&home, &procfs, &temp_dir, Some(100), &cut, test_now()).await;
    let artifact = String::from_utf8(obs.artifact_bytes).expect("utf8 artifact");
    assert!(
        !artifact.contains("No descendant processes found."),
        "must not print 'No descendant processes found.' when descendant collection failed: {artifact}"
    );
    assert!(
        artifact.contains("could not be read"),
        "must explain that descendants could not be read: {artifact}"
    );
}

#[tokio::test]
async fn filesystem_zero_total_blocks_or_inodes_records_unreadable_and_preserves_available_metric() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    // Case 1: total_blocks = 0, total_inodes = 100
    let statvfs_mock = move |_path: &Path| -> Result<StatvfsStats, rustix::io::Errno> {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 0,
            available_blocks: 0,
            total_inodes: 100,
            available_inodes: 20,
        })
    };

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, statvfs_mock).await;
    let home_fs = obs.summary.home_fs.expect("home_fs summary");
    assert_eq!(home_fs.space_percent, None, "zero total space must be None");
    assert_eq!(home_fs.inode_percent, Some(80), "available inodes must be preserved");
    assert!(
        obs.unreadable.iter().any(|f| f.section == SummarySection::Machine
            && f.part.contains("home filesystem space")
            && f.reason.contains("zero")),
        "must record unreadable failure for zero total space: {:?}",
        obs.unreadable
    );

    // Case 2: total_blocks = 100, total_inodes = 0
    let statvfs_mock_inodes_zero = move |_path: &Path| -> Result<StatvfsStats, rustix::io::Errno> {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 10,
            total_inodes: 0,
            available_inodes: 0,
        })
    };

    let params2 = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs2 = observe_with_statvfs(params2, statvfs_mock_inodes_zero).await;
    let home_fs2 = obs2.summary.home_fs.expect("home_fs summary");
    assert_eq!(home_fs2.space_percent, Some(90), "available space must be preserved");
    assert_eq!(home_fs2.inode_percent, None, "zero total inodes must be None");
    assert!(
        obs2.unreadable.iter().any(|f| f.section == SummarySection::Machine
            && f.part.contains("home filesystem inodes")
            && f.reason.contains("zero")),
        "must record unreadable failure for zero total inodes: {:?}",
        obs2.unreadable
    );
}

#[tokio::test]
async fn filesystem_available_greater_than_total_rejects_with_invalid_format() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let statvfs_mock = move |_path: &Path| -> Result<StatvfsStats, rustix::io::Errno> {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 150, // available > total
            total_inodes: 100,
            available_inodes: 50,
        })
    };

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, statvfs_mock).await;
    assert_eq!(obs.summary.home_fs, None, "invalid filesystem must not produce summary");
    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine
                && f.reason.contains("available space blocks exceed total blocks")),
        "must record unreadable failure when available blocks > total: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn read_proc_string_bounded_preserves_utf8_error_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let invalid_utf8_path = dir.path().join("invalid_utf8");
    fs::write(&invalid_utf8_path, [0xff, 0xfe, 0xfd]).expect("write bytes");

    let err = read_proc_string_bounded(&invalid_utf8_path, 1024).await.unwrap_err();
    match err {
        MachineError::Utf8 { path, source } => {
            assert_eq!(path, invalid_utf8_path);
            assert_eq!(source.utf8_error().valid_up_to(), 0);
        }
        other => panic!("expected MachineError::Utf8, got {other:?}"),
    }
}

#[test]
fn compute_process_age_preserves_concrete_reasons() {
    let now = Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(1_700_000_100).unwrap()).unwrap();
    let ticks = NonZeroU64::new(100);

    // Boot time unavailable
    let err_boot = super::compute_process_age(None, ticks, 1000, now).unwrap_err();
    assert!(matches!(err_boot, super::ProcessAgeError::BootTimeUnavailable));

    // Clock ticks unavailable
    let err_ticks = super::compute_process_age(Some(1_700_000_000), None, 1000, now).unwrap_err();
    assert!(matches!(err_ticks, super::ProcessAgeError::ClockTicksUnavailable));

    // Overflow calculating start seconds
    let err_overflow = super::compute_process_age(Some(u64::MAX), ticks, 1000, now).unwrap_err();
    assert!(matches!(err_overflow, super::ProcessAgeError::StartSecOverflow));

    // Success calculation
    let age = super::compute_process_age(Some(1_700_000_000), ticks, 2000, now).unwrap();
    assert_eq!(age, 80);
}

#[tokio::test]
async fn directory_size_bounded_detects_exceeded_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    for i in 0..5 {
        fs::write(base.join(format!("file_{i}.txt")), "data").expect("write file");
    }

    // Bound of 3 should trigger TooLarge error
    let err = super::directory_size_bounded(base, 3).await.unwrap_err();
    match err {
        MachineError::TooLarge { path, limit } => {
            assert_eq!(path, base);
            assert_eq!(limit, 3);
        }
        other => panic!("expected TooLarge error, got {other:?}"),
    }

    // Bound of 10 should succeed and calculate total size (5 files * 4 bytes = 20)
    let size = super::directory_size_bounded(base, 10).await.unwrap();
    assert_eq!(size, 20);
}

#[tokio::test]
async fn unrelated_proc_with_unreadable_cmdline_is_ignored_by_descendants() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    // Daemon PID 10
    write_proc_stat(&procfs, 10, 1);
    write_proc_cmdline(&procfs, 10, &["agent", "daemon"]);

    // Descendant PID 20 (parent 10)
    write_proc_stat(&procfs, 20, 10);
    write_proc_cmdline(&procfs, 20, &["worker", "child"]);

    // Unrelated PID 999 (parent 1, not under daemon)
    let unrelated_dir = procfs.join("999");
    fs::create_dir_all(&unrelated_dir).expect("create unrelated proc dir");
    write_proc_stat(&procfs, 999, 1);
    // Unreadable cmdline: create as a directory instead of a file so open/read fails with EISDIR
    fs::create_dir(unrelated_dir.join("cmdline")).expect("create dir as cmdline");

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: Some(10),
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, |_| {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 50,
            total_inodes: 100,
            available_inodes: 50,
        })
    })
    .await;

    // Descendants must contain PID 20 and NOT PID 999
    assert_eq!(obs.summary.daemon_descendants.len(), 1);
    assert_eq!(obs.summary.daemon_descendants[0].pid, 20);

    // Unreadable failures must NOT contain PID 999
    assert!(
        !obs.unreadable.iter().any(|f| f.part.contains("999")),
        "unrelated process must not generate read failures in unreadable: {:?}",
        obs.unreadable
    );
}

#[tokio::test]
async fn error_chain_preserves_concrete_os_cause_in_unreadable_and_artifact() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    // Create a tasks directory that fails on read_dir with PermissionDenied
    let tasks_dir = home.root().join("tasks");
    fs::create_dir_all(&tasks_dir).expect("create tasks");
    fs::set_permissions(&tasks_dir, std::fs::Permissions::from_mode(0o000)).expect("chmod 000");

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, |_| {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 50,
            total_inodes: 100,
            available_inodes: 50,
        })
    })
    .await;

    // Reset permissions for cleanup
    drop(fs::set_permissions(&tasks_dir, std::fs::Permissions::from_mode(0o755)));

    let failure = obs
        .unreadable
        .iter()
        .find(|f| f.part == "tasks directory")
        .expect("must report read failure for tasks directory");

    assert!(
        failure.reason.contains("cannot read") && failure.reason.contains("Permission denied"),
        "reason must preserve complete error chain including OS cause: {}",
        failure.reason
    );

    let artifact = String::from_utf8(obs.artifact_bytes).expect("artifact utf-8");
    assert!(
        artifact.contains("Permission denied"),
        "artifact must contain concrete OS cause: {artifact}"
    );
}

#[tokio::test]
async fn home_failure_preserves_same_device_temp_filesystem_and_anomaly() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let home_path = home.root().to_path_buf();
    let obs = observe_with_statvfs(params, move |path| {
        if path == home_path {
            Err(rustix::io::Errno::IO)
        } else {
            Ok(StatvfsStats {
                fragment_size: 4096,
                block_size: 4096,
                total_blocks: 100,
                available_blocks: 10,
                total_inodes: 100,
                available_inodes: 80,
            })
        }
    })
    .await;

    assert_eq!(
        obs.filesystems.len(),
        1,
        "failed home on same device must preserve successful temp filesystem"
    );
    assert_eq!(obs.filesystems[0].path, temp_dir);

    let assessment = Assessment {
        now: "2026-11-18T12:00:00Z".parse::<Timestamp>().unwrap(),
        stale_after: std::time::Duration::from_secs(3600),
        daemon: None,
        command_version: None,
        lead: "task-20261118-120000-lead".parse().unwrap(),
        tasks: vec![],
        items: vec![],
        lead_git: None,
        branches: vec![],
        filesystems: obs.filesystems,
        unanswered: vec![],
        unreadable: vec![],
    };
    let anomalies = assess(&assessment);
    assert!(
        anomalies.contains(&Anomaly::FullFilesystem(temp_dir)),
        "temp filesystem >= 90% full must trigger anomaly even when home fails: {anomalies:?}"
    );
}

#[tokio::test]
async fn directory_size_bounded_reports_too_many_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("file1"), b"hello").expect("write file1");
    fs::write(dir.path().join("file2"), b"world").expect("write file2");

    let result = directory_size_bounded(dir.path(), 1).await;
    match result {
        Err(MachineError::TooManyEntries { path, limit }) => {
            assert_eq!(path, dir.path());
            assert_eq!(limit, 1);
        }
        other => panic!("expected TooManyEntries, got {other:?}"),
    }
}

#[tokio::test]
async fn clock_ticks_helper_command_failure_reports_helper_error() {
    let result = query_clock_ticks_from(Path::new("/nonexistent/getconf-test-helper"), "CLK_TCK").await;
    assert!(
        matches!(result, Err(MachineError::Helper(_))),
        "nonexistent helper must report MachineError::Helper: {result:?}"
    );
}

#[tokio::test]
async fn clock_ticks_helper_exit_failure_reports_helper_exit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("fake-getconf");
    fs::write(&script, b"#!/bin/sh\nexit 1\n").expect("write script");
    fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let result = query_clock_ticks_from(&script, "CLK_TCK").await;
    assert!(
        matches!(result, Err(MachineError::HelperExit { .. })),
        "non-zero exit must report MachineError::HelperExit: {result:?}"
    );
}

#[tokio::test]
async fn clock_ticks_query_failure_records_ticks_err() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let daemon_pid = 100_u32;
    write_proc_stat(&procfs, 100, 1);
    write_proc_cmdline(&procfs, 100, &["daemon"]);
    write_proc_stat(&procfs, 200, 100);
    write_proc_cmdline(&procfs, 200, &["worker"]);

    let stat_content = "btime 1700000000\n";
    fs::write(procfs.join("stat"), stat_content).expect("write stat");

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: Some(daemon_pid),
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: None,
    };

    let fake_helper = PathBuf::from("/nonexistent/getconf-helper-failure");
    let obs = observe_with_statvfs_and_ticks_program(
        params,
        |_| {
            Ok(StatvfsStats {
                fragment_size: 4096,
                block_size: 4096,
                total_blocks: 100,
                available_blocks: 50,
                total_inodes: 100,
                available_inodes: 50,
            })
        },
        fake_helper,
    )
    .await;

    let failure = obs
        .unreadable
        .iter()
        .find(|f| f.section == SummarySection::Machine && f.part == "process age")
        .expect("must report read failure for process age when clock ticks query fails");

    assert!(
        failure.reason.contains("clock ticks per second unavailable"),
        "reason must report clock ticks unavailable: {}",
        failure.reason
    );
}

#[test]
fn process_age_pre_epoch_timestamp_reports_timestamp_before_epoch() {
    let pre_epoch = Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(-10).unwrap()).unwrap();
    let result = compute_process_age(Some(1000), NonZeroU64::new(100), 500, pre_epoch);
    assert!(
        matches!(result, Err(ProcessAgeError::TimestampBeforeEpoch { timestamp: -10 })),
        "timestamp before epoch must report TimestampBeforeEpoch: {result:?}"
    );
}

#[tokio::test]
async fn descendant_age_overflow_with_available_btime_and_ticks_records_unreadable() {
    let (dir, home, cut) = fixture();
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let daemon_pid = 100_u32;
    write_proc_stat(&procfs, 100, 1);
    write_proc_cmdline(&procfs, 100, &["daemon"]);
    write_proc_stat(&procfs, 200, 100);
    write_proc_cmdline(&procfs, 200, &["worker"]);

    // btime = u64::MAX causes boot.checked_add to overflow
    let stat_content = format!("btime {}\n", u64::MAX);
    fs::write(procfs.join("stat"), stat_content).expect("write stat");

    let params = MachineParams {
        home: &home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: Some(daemon_pid),
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, |_| {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 50,
            total_inodes: 100,
            available_inodes: 50,
        })
    })
    .await;

    let failure = obs
        .unreadable
        .iter()
        .find(|f| f.section == SummarySection::Machine && f.part == "process 200 age")
        .expect("must record per-process age failure when btime and ticks are available but age calculation overflows");

    assert!(
        failure.reason.contains("numeric overflow"),
        "reason must report numeric overflow: {}",
        failure.reason
    );
}

#[tokio::test]
async fn filesystem_metadata_failure_records_unreadable() {
    let (dir, _home, cut) = fixture();
    let blocked_dir = dir.path().join("blocked");
    fs::create_dir_all(&blocked_dir).expect("create blocked dir");
    fs::set_permissions(&blocked_dir, std::fs::Permissions::from_mode(0o000)).expect("chmod 000");

    let blocked_home = Home::new(
        &agent_core::config::AbsolutePath::try_from(blocked_dir.join("home").to_str().unwrap().to_owned()).unwrap(),
    );
    let procfs = dir.path().join("proc");
    fs::create_dir_all(&procfs).expect("create proc");
    let temp_dir = dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("create tmp");

    let params = MachineParams {
        home: &blocked_home,
        procfs: &procfs,
        temp_dir: &temp_dir,
        daemon_pid: None,
        cut: &cut,
        collected_at: test_now(),
        clock_ticks: NonZeroU64::new(100),
    };

    let obs = observe_with_statvfs(params, |_| {
        Ok(StatvfsStats {
            fragment_size: 4096,
            block_size: 4096,
            total_blocks: 100,
            available_blocks: 50,
            total_inodes: 100,
            available_inodes: 50,
        })
    })
    .await;

    drop(fs::set_permissions(
        &blocked_dir,
        std::fs::Permissions::from_mode(0o755),
    ));

    assert!(
        obs.unreadable
            .iter()
            .any(|f| f.section == SummarySection::Machine && f.part == "home filesystem metadata"),
        "inaccessible home parent must record home filesystem metadata failure: {:?}",
        obs.unreadable
    );
}
