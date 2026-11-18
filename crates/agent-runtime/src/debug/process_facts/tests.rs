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

#[test]
fn rejects_malformed_or_overflowing_process_start_fields() {
    let ticks = std::num::NonZeroU64::new(100).unwrap();
    assert!(matches!(
        start_time("broken", "btime 0\n", ticks),
        Err(ProcessError::Format("process name is missing"))
    ));
    assert!(matches!(
        start_time("1 (x) S", "btime 0\n", ticks),
        Err(ProcessError::Format("start ticks are missing"))
    ));
    let fields = (3..=21).map(|_| "0").collect::<Vec<_>>().join(" ");
    let stat = format!("1 (x) {fields} 0");
    assert!(matches!(
        start_time(&stat, "cpu 0\n", ticks),
        Err(ProcessError::Format("boot time is missing"))
    ));
    assert!(matches!(
        start_time(&stat, "btime invalid\n", ticks),
        Err(ProcessError::Number(_))
    ));
    assert!(matches!(
        start_time(&stat, "btime 18446744073709551615\n", ticks),
        Err(ProcessError::Range(_))
    ));
}

#[tokio::test]
async fn unreadable_procfs_is_a_failure_with_the_source_path() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    std::fs::write(home.lock_file(), "").unwrap();
    let missing = dir.path().join("proc");
    let result = observe_with_ticks(&home, &missing, std::num::NonZeroU64::new(100).unwrap()).await;
    assert!(
        matches!(result, Err(ProcessError::Io {path, source}) if path == missing.join("locks") && source.kind() == std::io::ErrorKind::NotFound)
    );
}

#[tokio::test]
async fn rejects_procfs_bytes_over_the_read_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(LIMIT + 1).unwrap();
    assert!(matches!(bytes(&path).await, Err(ProcessError::TooLarge)));
}

#[test]
fn resolves_replaced_executable_link_to_its_current_disk_path() {
    for (link, links, expected) in [
        ("/tmp/café (deleted)", 0, "/tmp/café"),
        ("/tmp/café (deleted)", 1, "/tmp/café (deleted)"),
        ("/tmp/agent", 1, "/tmp/agent"),
    ] {
        assert_eq!(disk_path(Path::new(link), links).unwrap(), PathBuf::from(expected));
    }
}
