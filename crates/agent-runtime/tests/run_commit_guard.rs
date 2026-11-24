//! The hooks of an agent's run: the `git` a run starts is pointed at a hooks directory whose `commit-msg` asks the harness
//! (`agent guard-commit`) about every commit, for every CLI, without a change to the repository.

#[cfg(test)]
mod run_commit_guard {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use agent_core::cli::{RunMode, SessionId};
    use agent_core::config::{AbsolutePath, CliArgs, Config, Language, Secrets};
    use agent_core::ids::Clock;
    use agent_core::kinds::CliKind;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::commit_guard::COMMIT_REFUSED_STATUS;
    use agent_runtime::git::CommitIdentity;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, RunEngine, RunJob, RunTimings, RunVerdict};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug, Clone)]
    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            "2026-11-12T10:00:00Z"
                .parse::<agent_core::state::Timestamp>()
                .unwrap()
                .as_utc()
        }
    }

    fn absolute(path: &Path) -> AbsolutePath {
        path.to_str().unwrap().parse().unwrap()
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// A run of the fake `claude` that tries to commit an edit of a frozen test, in a repository with one commit, with
    /// the stand-in for `agent` that ends with `code`; returns the repository, the file the stand-in wrote its calls to and
    /// the temporary directories.
    struct Trial {
        _dirs: Vec<tempfile::TempDir>,
        tree: PathBuf,
        calls: PathBuf,
    }

    async fn trial(code: i32) -> Trial {
        let (temp, tree_dir, bin) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let tree = tree_dir.path().canonicalize().unwrap();
        git(&tree, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(tree.join("tests")).unwrap();
        std::fs::write(tree.join("tests/a.rs"), "one\ntwo\n").unwrap();
        git(&tree, &["add", "-A"]);
        git(&tree, &["commit", "-q", "-m", "RED: a test"]);
        let calls = bin.path().join("calls");
        let stand_in = bin.path().join("agent");
        std::fs::write(
            &stand_in,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit {code}\n", calls.display()),
        )
        .unwrap();
        std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
        let home = Home::new(&absolute(temp.path()));
        let store = Arc::new(StateStore::open(home).await.unwrap());
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run/fake-claude.sh");
        let config = Config::from_toml_str(&format!("[binaries]\nclaude = \"{}\"\n", fake.display())).unwrap();
        let texts = HarnessPrompts::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            &Language::default(),
        )
        .await
        .unwrap();
        let engine = RunEngine::new(EngineParts {
            store,
            config: Arc::new(config),
            texts: Arc::new(texts),
            secrets: Arc::new(Secrets::default()),
            supervisor: Supervisor::new(NonZeroUsize::MIN),
            clock: SharedClock::new(FixedClock),
            local_offset: time::UtcOffset::UTC,
            timings: RunTimings {
                poll: Duration::from_millis(20),
                hang: Duration::from_secs(20),
                stop_grace: Duration::from_secs(5),
            },
            agent_binary_dir: absolute(bin.path()),
        });
        std::fs::write(tree.join(".fake-scenario"), "commit-frozen-test").unwrap();
        let session_id: SessionId = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07".parse().unwrap();
        let job = RunJob {
            task: "task-20261112-100000-t1t1".parse().unwrap(),
            run: "run-20261112-100001-r1r1".parse().unwrap(),
            cli: CliKind::Claude,
            model: None,
            extra_args: CliArgs::default(),
            mode: RunMode::New { session_id },
            prompt: "Commit.".to_owned(),
            working_dir: absolute(&tree),
            identity: CommitIdentity::new("Fixture".to_owned(), "fixture@example.com".to_owned(), None),
            mcp_servers: BTreeMap::new(),
            context: None,
            output: None,
        };
        let RunVerdict::Finished(_) = engine.run(job, &CancellationToken::new()).await.unwrap() else {
            panic!("the fake CLI must finish");
        };
        Trial {
            _dirs: vec![temp, tree_dir, bin],
            tree,
            calls,
        }
    }

    // Rule: the git a run starts asks the guard about each commit, and a refusal stops the commit.
    #[tokio::test]
    async fn a_commit_of_a_run_is_stopped_when_the_guard_refuses() {
        let trial = trial(i32::from(COMMIT_REFUSED_STATUS)).await;

        let code = std::fs::read_to_string(trial.tree.join(".fake-commit-code")).unwrap_or_default();
        let asked = std::fs::read_to_string(&trial.calls).unwrap_or_default();

        assert_eq!(code.trim(), "1");
        assert!(asked.starts_with("guard-commit "), "{asked}");
        assert_eq!(git(&trial.tree, &["rev-list", "--count", "HEAD"]), "1");
    }

    // Rule: a commit the guard allows is made.
    #[tokio::test]
    async fn a_commit_of_a_run_is_made_when_the_guard_allows() {
        let trial = trial(0).await;

        let code = std::fs::read_to_string(trial.tree.join(".fake-commit-code")).unwrap_or_default();

        assert_eq!(code.trim(), "0");
        assert_eq!(git(&trial.tree, &["rev-list", "--count", "HEAD"]), "2");
    }

    // Rule: the repository is not changed: no hooks path is written into its configuration.
    #[tokio::test]
    async fn the_configuration_of_the_repository_is_not_changed() {
        let trial = trial(i32::from(COMMIT_REFUSED_STATUS)).await;

        let configured = Command::new("git")
            .args(["config", "--local", "--get", "core.hooksPath"])
            .current_dir(&trial.tree)
            .output()
            .unwrap();

        assert_eq!(configured.status.code(), Some(1));
        assert!(!asked_hooks_dir_in_repo(&trial.tree));
    }

    /// Whether the repository's own hooks directory got a `commit-msg`.
    fn asked_hooks_dir_in_repo(tree: &Path) -> bool {
        tree.join(".git/hooks/commit-msg").exists()
    }
}
