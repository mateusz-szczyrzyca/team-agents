//! The switch of the early refusal of a commit of a frozen test, `commit_guard` under `[repos.NAME.checks]`, through the
//! public API: on by default, off with `false`, and the checks of the table are the same either way.

#[cfg(test)]
mod config_commit_guard {
    use agent_core::config::{CommitGuard, Config, ConfigError};

    /// A configuration with one agent and the repository `team-agents`, followed by `extra`.
    fn config_with(extra: &str) -> String {
        format!(
            "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n\
             [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n{extra}"
        )
    }

    fn guard_of(text: &str) -> CommitGuard {
        let config = Config::from_toml_str(text).unwrap_or_else(|error| panic!("{error:?}"));
        config.repo(&"team-agents".parse().unwrap()).unwrap().commit_guard
    }

    // Rule: the guard is on unless the table says otherwise, also without a checks table.
    #[test]
    fn the_guard_is_on_by_default() {
        assert_eq!(guard_of(&config_with("")), CommitGuard::Enabled);
        assert_eq!(
            guard_of(&config_with("[repos.team-agents.checks]\ntest_paths = [\"tests/\"]\n")),
            CommitGuard::Enabled
        );
        assert_eq!(
            guard_of(&config_with("[repos.team-agents.checks]\ncommit_guard = true\n")),
            CommitGuard::Enabled
        );
    }

    // Rule: `commit_guard = false` turns the guard off and leaves the lists of the table as they are.
    #[test]
    fn false_turns_the_guard_off_and_keeps_the_lists() {
        let text = config_with("[repos.team-agents.checks]\ntest_paths = [\"tests/\"]\ncommit_guard = false\n");
        let config = Config::from_toml_str(&text).unwrap();
        let repo = config.repo(&"team-agents".parse().unwrap()).unwrap();

        assert_eq!(repo.commit_guard, CommitGuard::Disabled);
        let checks = repo.checks.as_ref().unwrap();
        assert!(checks.test_paths.matches(&"tests/a.rs".parse().unwrap()));
    }

    // Rule: the key is a boolean.
    #[test]
    fn a_value_that_is_no_boolean_is_refused() {
        for value in ["\"no\"", "0", "[true]"] {
            let text = config_with(&format!("[repos.team-agents.checks]\ncommit_guard = {value}\n"));

            let error = Config::from_toml_str(&text).unwrap_err();

            assert!(matches!(error, ConfigError::Parse(_)), "{value}: {error:?}");
        }
    }

    // Rule: what users copy loads, and the example names the key with its default.
    #[test]
    fn the_example_configuration_names_the_switch() {
        let example = include_str!("../../../config/agents.example.toml");

        let config = Config::from_toml_str(example).unwrap();

        assert!(example.contains("commit_guard = true"));
        assert_eq!(
            config.repo(&"team-agents".parse().unwrap()).unwrap().commit_guard,
            CommitGuard::Enabled
        );
    }
}
