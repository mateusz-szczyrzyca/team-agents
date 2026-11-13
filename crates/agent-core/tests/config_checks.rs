//! The `checks` table of a repository and `check_passes` of `[review]` through the public API: what the checks before a
//! review take for tests, for production code and for what judges a round, and how often they send a round back.

#[cfg(test)]
mod config_checks {
    use agent_core::config::{Config, ConfigError};
    use agent_core::repo_path::{MAX_PATTERNS, RepoPath};

    /// A configuration with one agent, two investigators and one repository, followed by `extra`.
    fn config_with(extra: &str) -> String {
        format!(
            "[agents.claude-1]\ncli = \"claude\"\n[investigators.inv-1]\ncli = \"claude\"\n\
             [investigators.inv-2]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n{extra}"
        )
    }

    /// The repository `team-agents` with `[repos.team-agents.checks]` and its `body`.
    fn repo_with_checks(body: &str) -> String {
        config_with(&format!(
            "[repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n[repos.team-agents.checks]\n{body}"
        ))
    }

    fn path(text: &str) -> RepoPath {
        text.parse().unwrap()
    }

    fn checks_of(text: &str) -> agent_core::config::Checks {
        let config = Config::from_toml_str(text).unwrap();
        config
            .repo(&"team-agents".parse().unwrap())
            .unwrap()
            .checks
            .clone()
            .expect("the repository has a checks table")
    }

    // Rule: a repository without the table has no checks: its rounds go straight to review.
    #[test]
    fn a_repository_without_the_table_has_no_checks() {
        let config = Config::from_toml_str(&config_with(
            "[repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
        ))
        .unwrap();

        let repo = config.repo(&"team-agents".parse().unwrap()).unwrap();

        assert_eq!(repo.checks, None);
    }

    // Rule: the three lists tell tests, production code and what judges a round apart, each as path patterns.
    #[test]
    fn the_table_lists_the_tests_the_code_and_what_judges_a_round() {
        let checks = checks_of(&repo_with_checks(
            "test_paths = [\"tests/\", \"*_test.go\"]\ncode_paths = [\"crates/\"]\nprotected_paths = [\".github/\", \"AGENTS.md\"]\n",
        ));

        assert!(checks.test_paths.matches(&path("tests/a.rs")));
        assert!(checks.test_paths.matches(&path("agentbus/x_test.go")));
        assert!(!checks.test_paths.matches(&path("crates/a.rs")));
        assert!(checks.code_paths.matches(&path("crates/a/src/lib.rs")));
        assert!(checks.protected_paths.matches(&path(".github/workflows/ci.yml")));
        assert!(checks.protected_paths.matches(&path("docs/AGENTS.md")));
        assert_eq!(checks.test_paths.as_slice().len(), 2);
    }

    #[test]
    fn a_list_that_is_left_out_is_empty() {
        let checks = checks_of(&repo_with_checks("code_paths = [\"src/\"]\n"));

        assert_eq!(checks.test_paths.as_slice(), []);
        assert_eq!(checks.protected_paths.as_slice(), []);
        assert_eq!(checks.code_paths.as_slice().len(), 1);
    }

    #[test]
    fn an_empty_table_is_a_table_with_three_empty_lists() {
        let checks = checks_of(&repo_with_checks(""));

        assert_eq!(checks, agent_core::config::Checks::default());
    }

    // Rule: the table is strict like the rest of the file.
    #[test]
    fn a_key_the_table_does_not_know_is_refused() {
        let error = Config::from_toml_str(&repo_with_checks("test_path = [\"tests/\"]\n")).unwrap_err();

        assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
    }

    #[test]
    fn a_pattern_that_breaks_a_rule_is_refused_with_the_file_not_loading() {
        for body in [
            "test_paths = [\"/tests\"]\n",
            "test_paths = [\"a[b].rs\"]\n",
            "code_paths = [\"\"]\n",
            "protected_paths = [\"a/../b\"]\n",
            "test_paths = \"tests/\"\n",
        ] {
            let error = Config::from_toml_str(&repo_with_checks(body)).unwrap_err();

            assert!(matches!(error, ConfigError::Parse(_)), "{body}: {error:?}");
        }
    }

    #[test]
    fn a_list_may_have_as_many_patterns_as_the_limit_and_no_more() {
        let list = |count: usize| {
            let patterns: Vec<String> = (0..count).map(|n| format!("\"dir{n}/\"")).collect();
            format!("test_paths = [{}]\n", patterns.join(", "))
        };

        assert_eq!(
            checks_of(&repo_with_checks(&list(MAX_PATTERNS)))
                .test_paths
                .as_slice()
                .len(),
            MAX_PATTERNS
        );
        let error = Config::from_toml_str(&repo_with_checks(&list(MAX_PATTERNS + 1))).unwrap_err();
        assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
    }

    // Rule: `check_passes` is a count of passes like `fix_passes`: 2 unless the file says another, 0 asks at once.
    #[test]
    fn the_checks_send_a_round_back_twice_unless_the_file_says_another_number() {
        let config = Config::from_toml_str(&config_with("")).unwrap();

        assert_eq!(config.review().check_passes, 2);
        assert_eq!(config.review().fix_passes, 5);
    }

    #[test]
    fn the_file_sets_the_passes_of_the_checks_apart_from_those_of_the_reviews() {
        let config = Config::from_toml_str(&config_with("[review]\ncheck_passes = 7\n")).unwrap();

        assert_eq!(config.review().check_passes, 7);
        assert_eq!(config.review().fix_passes, 5);
    }

    #[test]
    fn zero_check_passes_ask_the_human_at_once() {
        let config = Config::from_toml_str(&config_with("[review]\ncheck_passes = 0\nfix_passes = 1\n")).unwrap();

        assert_eq!(config.review().check_passes, 0);
        assert_eq!(config.review().fix_passes, 1);
    }

    #[test]
    fn a_negative_or_textual_number_of_check_passes_is_refused() {
        for text in ["check_passes = -1", "check_passes = \"2\"", "check_passes = 2.5"] {
            let error = Config::from_toml_str(&config_with(&format!("[review]\n{text}\n"))).unwrap_err();

            assert!(matches!(error, ConfigError::Parse(_)), "{text}: {error:?}");
        }
    }

    // Rule: what users copy loads: the example describes the table and the number.
    #[test]
    fn the_example_configuration_has_the_checks_of_its_repository() {
        let example = include_str!("../../../config/agents.example.toml");
        let config = Config::from_toml_str(example).unwrap();

        let repo = config.repo(&"team-agents".parse().unwrap()).unwrap();
        let checks = repo.checks.as_ref().expect("the example has a checks table");

        assert!(checks.test_paths.matches(&path("tests/unit/watchdog.bats")));
        assert!(checks.protected_paths.matches(&path(".github/workflows/ci.yml")));
        assert!(checks.protected_paths.matches(&path("AGENTS.md")));
        assert!(checks.code_paths.matches(&path("crates/agent-core/src/lib.rs")));
        assert_eq!(config.review().check_passes, 2);
    }
}
