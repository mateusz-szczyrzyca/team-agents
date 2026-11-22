//! The hooks directory of an agent's runs, with real git: the guard is asked about every commit, a refusal stops it, and
//! the repository's own hooks go on running.

#[cfg(test)]
mod commit_guard_hooks {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_runtime::commit_guard::hooks::install_hooks;
    use agent_runtime::git::{CommitIdentity, Git};

    fn git_in(dir: &Path, hooks: Option<&Path>, args: &[&str]) -> Output {
        let mut command = Command::new("git");
        command.args(["-c", "commit.gpgsign=false"]);
        if let Some(hooks) = hooks {
            command.arg("-c").arg(format!("core.hooksPath={}", hooks.display()));
        }
        command
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap()
    }

    fn handle(dir: &Path) -> Git {
        Git::new(
            AbsolutePath::from_str(dir.to_str().unwrap()).unwrap(),
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    fn executable(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    struct Repository {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        repo: PathBuf,
    }

    fn repository() -> Repository {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert!(git_in(&repo, None, &["init", "-q", "-b", "main"]).status.success());
        Repository { _tmp: tmp, root, repo }
    }

    /// A stand-in for `agent`: it writes its arguments to `calls` and ends with `code`.
    fn stand_in(dir: &Path, calls: &Path, code: i32) -> PathBuf {
        let path = dir.join("agent");
        executable(
            &path,
            &format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit {code}\n", calls.display()),
        );
        path
    }

    fn commit(repo: &Path, hooks: &Path, message: &str) -> Output {
        std::fs::write(repo.join("file.txt"), message).unwrap();
        assert!(git_in(repo, None, &["add", "-A"]).status.success());
        git_in(repo, Some(hooks), &["commit", "-q", "-m", message])
    }

    fn commits(repo: &Path) -> usize {
        let output = git_in(repo, None, &["rev-list", "--count", "--all"]);
        String::from_utf8(output.stdout).unwrap().trim().parse().unwrap()
    }

    // Rule: the guard is asked about each commit with the file that holds its message, and a refusal stops the commit.
    #[tokio::test]
    async fn the_guard_is_asked_with_the_message_file_and_its_refusal_stops_the_commit() {
        let world = repository();
        let calls = world.root.join("calls");
        let agent = stand_in(&world.root, &calls, 1);
        let hooks = install_hooks(&handle(&world.repo), &world.root.join("hooks"), &agent)
            .await
            .unwrap();

        let output = commit(&world.repo, &hooks, "Format the test");

        assert!(!output.status.success());
        assert_eq!(commits(&world.repo), 0);
        let asked = std::fs::read_to_string(&calls).unwrap_or_default();
        assert!(asked.starts_with("guard-commit "), "{asked}");
        assert!(asked.trim_end().ends_with("COMMIT_EDITMSG"), "{asked}");
    }

    // Rule: a commit the guard lets through is made.
    #[tokio::test]
    async fn a_commit_the_guard_allows_is_made() {
        let world = repository();
        let calls = world.root.join("calls");
        let agent = stand_in(&world.root, &calls, 0);
        let hooks = install_hooks(&handle(&world.repo), &world.root.join("hooks"), &agent)
            .await
            .unwrap();

        let output = commit(&world.repo, &hooks, "Cover one more case");

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(commits(&world.repo), 1);
    }

    // Rule: the repository's own hooks keep running: the other hooks are linked in, its commit-msg runs after the guard
    // allows and not when the guard refuses, and the samples are left out.
    #[tokio::test]
    async fn the_hooks_of_the_repository_keep_running() {
        let world = repository();
        let hooks_of_repo = world.repo.join(".git/hooks");
        executable(
            &hooks_of_repo.join("pre-commit"),
            &format!("#!/bin/sh\necho pre >> '{}'\n", world.root.join("seen").display()),
        );
        executable(
            &hooks_of_repo.join("commit-msg"),
            &format!(
                "#!/bin/sh\necho \"msg $1\" >> '{}'\n",
                world.root.join("seen").display()
            ),
        );
        executable(&hooks_of_repo.join("post-merge.sample"), "#!/bin/sh\n");
        let calls = world.root.join("calls");
        let allowing = stand_in(&world.root, &calls, 0);
        let hooks = install_hooks(&handle(&world.repo), &world.root.join("hooks"), &allowing)
            .await
            .unwrap();

        let made = commit(&world.repo, &hooks, "One");
        let seen = std::fs::read_to_string(world.root.join("seen")).unwrap_or_default();

        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        assert!(seen.starts_with("pre\nmsg "), "{seen}");
        assert!(!hooks.join("post-merge.sample").exists());

        std::fs::remove_file(world.root.join("seen")).unwrap();
        let refusing = stand_in(&world.root, &calls, 1);
        let hooks = install_hooks(&handle(&world.repo), &world.root.join("hooks"), &refusing)
            .await
            .unwrap();

        let refused = commit(&world.repo, &hooks, "Two");
        let seen = std::fs::read_to_string(world.root.join("seen")).unwrap_or_default();

        assert!(!refused.status.success());
        assert_eq!(seen, "pre\n");
    }

    // Rule: a path with a quote in it is written into the hook as it is.
    #[tokio::test]
    async fn a_path_with_a_quote_is_written_as_it_is() {
        let world = repository();
        let calls = world.root.join("calls");
        let quoted = world.root.join("it's here");
        std::fs::create_dir(&quoted).unwrap();
        let agent = stand_in(&quoted, &calls, 0);
        let hooks = install_hooks(&handle(&world.repo), &world.root.join("hooks"), &agent)
            .await
            .unwrap();

        let output = commit(&world.repo, &hooks, "Quoted");

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(
            std::fs::read_to_string(&calls)
                .unwrap_or_default()
                .starts_with("guard-commit ")
        );
    }
}
