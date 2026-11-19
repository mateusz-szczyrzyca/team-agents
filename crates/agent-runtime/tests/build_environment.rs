//! The managed Cargo policy reaches the real child process, including resumed runs.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use agent_core::cli::{RunMode, SessionId};
    use agent_core::config::{AbsolutePath, CliArgs, Config, Language, Secrets};
    use agent_core::ids::Clock;
    use agent_core::kinds::CliKind;
    use agent_core::state::RunEnd;
    use agent_core::status::RunOutcome;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::git::CommitIdentity;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, RunEngine, RunJob, RunTimings, RunVerdict};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// A fixed instant makes run records independent of the wall clock.
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

    #[tokio::test]
    async fn agent_children_use_configured_incremental_policy_over_secret_environment() {
        for (setting, expected, conflicting) in [
            ("", "0", "1"),
            ("[build]\nincremental = false", "0", "1"),
            ("[build]\nincremental = true", "1", "0"),
        ] {
            for resumed in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let tree = tempfile::tempdir().unwrap();
                let home = Home::new(&absolute(temp.path()));
                let store = Arc::new(StateStore::open(home).await.unwrap());
                let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run/fake-claude.sh");
                let config =
                    Config::from_toml_str(&format!("[binaries]\nclaude = \"{}\"\n{setting}\n", fake.display()))
                        .unwrap();
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
                    secrets: Arc::new(
                        Secrets::parse(&format!("[agent_env]\nCARGO_INCREMENTAL = \"{conflicting}\"\n"), 0o600)
                            .unwrap(),
                    ),
                    supervisor: Supervisor::new(NonZeroUsize::MIN),
                    clock: SharedClock::new(FixedClock),
                    local_offset: time::UtcOffset::UTC,
                    timings: RunTimings {
                        poll: Duration::from_millis(20),
                        hang: Duration::from_secs(20),
                        stop_grace: Duration::from_secs(5),
                    },
                    agent_binary_dir: absolute(Path::new("/usr/bin")),
                });
                std::fs::write(tree.path().join(".fake-scenario"), "env-incremental").unwrap();
                let session_id: SessionId = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07".parse().unwrap();
                let job = RunJob {
                    task: "task-20261112-100000-t1t1".parse().unwrap(),
                    run: "run-20261112-100001-r1r1".parse().unwrap(),
                    cli: CliKind::Claude,
                    model: None,
                    extra_args: CliArgs::default(),
                    mode: if resumed {
                        RunMode::Resume { session_id }
                    } else {
                        RunMode::New { session_id }
                    },
                    prompt: "Record the environment.".to_owned(),
                    working_dir: absolute(tree.path()),
                    identity: CommitIdentity::new("Fixture".to_owned(), "fixture@example.com".to_owned(), None),
                    mcp_servers: BTreeMap::new(),
                };
                let RunVerdict::Finished(run) = engine.run(job, &CancellationToken::new()).await.unwrap() else {
                    panic!("the fake CLI must finish");
                };
                assert_eq!(
                    run.classified.end,
                    RunEnd::Reported {
                        outcome: RunOutcome::Done
                    }
                );
                assert_eq!(
                    std::fs::read_to_string(tree.path().join(".fake-incremental")).unwrap(),
                    format!("{expected}\n")
                );
            }
        }
    }
}
