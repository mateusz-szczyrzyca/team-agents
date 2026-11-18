//! Contracts for process selection, byte boundaries, and procfs parsing.
use super::*;
use agent_core::config::AbsolutePath;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

#[tokio::test]
async fn reads_actual_lock_holder_start_arguments_names_and_executable() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    std::fs::write(home.lock_file(), "").unwrap();
    let meta = std::fs::metadata(home.lock_file()).unwrap();
    let procfs = dir.path().join("proc");
    std::fs::create_dir_all(procfs.join("42")).unwrap();
    std::fs::write(
        procfs.join("locks"),
        format!(
            "1: FLOCK ADVISORY WRITE 77 00:00:1 0 EOF\n2: FLOCK ADVISORY WRITE 42 {:x}:{:x}:{} 0 EOF\n",
            rustix::fs::major(meta.dev()),
            rustix::fs::minor(meta.dev()),
            meta.ino()
        ),
    )
    .unwrap();
    std::fs::write(procfs.join("stat"), "cpu 0 0\nbtime 1735689600\n").unwrap();
    let fields = (3..=21).map(|_| "0").collect::<Vec<_>>().join(" ");
    std::fs::write(
        procfs.join("42/stat"),
        format!("42 (daemon with ) spaces) {fields} 1200 0\n"),
    )
    .unwrap();
    std::fs::write(procfs.join("42/cmdline"), b"agent\0daemon\0--home\0caf\xc3\xa9\0").unwrap();
    std::fs::write(procfs.join("42/environ"), b"TOKEN=never-copy-this\0LANG=en_US.UTF-8\0").unwrap();
    let exe = dir.path().join("version");
    std::fs::write(&exe, "#!/bin/sh\nprintf 'agent 0.1.0\\n'\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&exe, procfs.join("42/exe")).unwrap();
    let observed = observe_with_ticks(&home, &procfs, std::num::NonZeroU64::new(100).unwrap())
        .await
        .unwrap();
    assert!(observed.is_some(), "the matching lock owner must be collected");
    let observed = observed.unwrap();
    assert_eq!(observed.pid, 42);
    assert_eq!(observed.started_at, Some("2025-01-01T00:00:12Z".parse().unwrap()));
    assert_eq!(observed.cmdline, vec!["agent", "daemon", "--home", "café"]);
    assert_eq!(observed.env_var_names, vec!["LANG", "TOKEN"]);
    assert_eq!(observed.exe_path, exe.to_str().unwrap());
    assert_eq!(observed.version, "agent 0.1.0");
    assert!(observed.disk_newer_than_process);
}

#[tokio::test]
async fn missing_home_lock_means_no_daemon_without_creating_a_lock() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    assert_eq!(observe(&home, &dir.path().join("no-proc")).await.unwrap(), None);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
