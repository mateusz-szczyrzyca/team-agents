use std::path::Path;

use agent_core::config::AbsolutePath;

use super::*;

#[test]
fn fake_free_space_returns_configured_bytes() {
    let space = FakeFreeSpace::new(50 * 1024 * 1024 * 1024);
    assert_eq!(space.free_space_bytes(Path::new("/")).unwrap(), 50 * 1024 * 1024 * 1024);
    space.set(20 * 1024 * 1024 * 1024);
    assert_eq!(space.free_space_bytes(Path::new("/")).unwrap(), 20 * 1024 * 1024 * 1024);
}

#[test]
fn remove_build_directory_removes_target_and_returns_released_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let abs = AbsolutePath::try_from(temp.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    let repo_dir = home.worktrees_dir().join("test-repo");
    let task_dir = repo_dir.join("task-1");
    let target_dir = task_dir.join("target");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(target_dir.join("binary"), vec![0u8; 1024]).unwrap();

    let released = remove_build_directory(&home, &task_dir).unwrap();
    assert_eq!(released, 1024);
    assert!(!target_dir.exists());
    assert!(task_dir.exists());
}

#[test]
fn remove_build_directory_refuses_paths_outside_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let abs = AbsolutePath::try_from(temp.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    let outside = temp.path().join("other").join("task-1");
    std::fs::create_dir_all(&outside).unwrap();

    let err = remove_build_directory(&home, &outside).unwrap_err();
    assert!(matches!(err, BuildDirError::OutsideHome { .. }));
}

#[test]
#[cfg(unix)]
fn remove_build_directory_refuses_symlink_target() {
    let temp = tempfile::tempdir().unwrap();
    let abs = AbsolutePath::try_from(temp.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    let repo_dir = home.worktrees_dir().join("test-repo");
    let task_dir = repo_dir.join("task-1");
    std::fs::create_dir_all(&task_dir).unwrap();
    let external = temp.path().join("external");
    std::fs::create_dir_all(&external).unwrap();
    std::os::unix::fs::symlink(&external, task_dir.join("target")).unwrap();

    let err = remove_build_directory(&home, &task_dir).unwrap_err();
    assert!(matches!(err, BuildDirError::Symlink { .. }));
}

#[test]
fn largest_worktrees_finds_top_five_ordered_by_size() {
    let temp = tempfile::tempdir().unwrap();
    let abs = AbsolutePath::try_from(temp.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    let repo_dir = home.worktrees_dir().join("test-repo");
    for i in 1..=6 {
        let task_dir = repo_dir.join(format!("task-{i}"));
        std::fs::create_dir_all(&task_dir).unwrap();
        std::fs::write(task_dir.join("file"), vec![0u8; i * 100]).unwrap();
    }

    let top = largest_worktrees(&home, 5).unwrap();
    assert_eq!(top.len(), 5);
    assert_eq!(top[0].bytes, 600);
    assert_eq!(top[1].bytes, 500);
    assert_eq!(top[2].bytes, 400);
    assert_eq!(top[3].bytes, 300);
    assert_eq!(top[4].bytes, 200);
}

#[test]
#[cfg(unix)]
fn remove_build_directory_refuses_a_worktree_that_links_outside_home() {
    let temp = tempfile::tempdir().unwrap();
    let abs = AbsolutePath::try_from(temp.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    let repo_dir = home.worktrees_dir().join("test-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let external = temp.path().join("external");
    std::fs::create_dir_all(external.join("target")).unwrap();
    std::fs::write(external.join("target").join("binary"), b"build output").unwrap();
    let linked_worktree = repo_dir.join("task-1");
    std::os::unix::fs::symlink(&external, &linked_worktree).unwrap();

    let err = remove_build_directory(&home, &linked_worktree).unwrap_err();

    assert!(matches!(err, BuildDirError::OutsideHome { .. }));
    assert!(external.join("target").join("binary").exists());
}

#[test]
fn remove_build_directory_of_a_missing_worktree_releases_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let abs = AbsolutePath::try_from(temp.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    std::fs::create_dir_all(home.worktrees_dir().join("test-repo")).unwrap();

    let released = remove_build_directory(&home, &home.worktrees_dir().join("test-repo").join("gone")).unwrap();

    assert_eq!(released, 0);
}

#[test]
fn gibibytes_text_rounds_down_to_hundredths() {
    assert_eq!(gibibytes_text(0), "0.00 GB");
    assert_eq!(gibibytes_text(GIBIBYTE + 5 * 1024 * 1024), "1.00 GB");
    assert_eq!(gibibytes_text(GIBIBYTE + 300 * 1024 * 1024), "1.29 GB");
    assert_eq!(gibibytes_text(34 * GIBIBYTE), "34.00 GB");
}
