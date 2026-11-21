//! The tests of the keys of `[replacement]` that item 1.2b adds: the agents that may take a task over, the mode and the
//! number of takeovers a task gets. The keys of the new sessions are tested in `config/replacement_tests.rs`.

#[cfg(test)]
mod takeover {
    use crate::config::{Config, ConfigError};
    use crate::decide::replacement::ReplacementMode;
    use crate::ids::AgentName;

    /// Two normal agents and one worker, which is all these tests need.
    fn agents() -> &'static str {
        "[agents.claude-1]\ncli = \"claude\"\n\n[agents.claude-2]\ncli = \"claude\"\n\n[agents.glm]\ncli = \"opencode\"\n\n\
         [tiers]\nnormal = [\"claude-1\", \"claude-2\"]\nworkers = [\"glm\"]\n"
    }

    /// The configuration of [`agents`] with `table` after it.
    fn with(table: &str) -> String {
        format!("{}{table}", agents())
    }

    /// The names of `list` as text.
    fn names(list: &[AgentName]) -> Vec<&str> {
        list.iter().map(AgentName::as_str).collect()
    }

    #[test]
    fn a_file_without_replacement_starts_a_new_session_once_and_names_no_agent_to_take_over() {
        let config = Config::from_toml_str(agents()).unwrap();

        assert_eq!(config.replacement().new_sessions().get(), 1);
        assert_eq!(config.replacement().replacements().get(), 1);
        assert_eq!(config.replacement().mode(), ReplacementMode::Auto);
        assert_eq!(config.replacement().agents(), Vec::<AgentName>::new());
    }

    #[test]
    fn the_replacement_table_says_how_many_new_sessions_and_takeovers_a_task_has() {
        let config = Config::from_toml_str(&with("[replacement]\nnew_sessions = 3\nreplacements = 2\n")).unwrap();
        assert_eq!(config.replacement().new_sessions().get(), 3);
        assert_eq!(config.replacement().replacements().get(), 2);

        let off = Config::from_toml_str(&with("[replacement]\nreplacements = 0\n")).unwrap();
        assert_eq!(off.replacement().replacements().get(), 0);
    }

    #[test]
    fn the_replacement_table_names_the_agents_that_may_take_a_task_over_in_order() {
        let config = Config::from_toml_str(&with("[replacement]\nagents = [\"claude-2\", \"glm\"]\n")).unwrap();

        assert_eq!(names(config.replacement().agents()), ["claude-2", "glm"]);
    }

    #[test]
    fn the_replacement_table_says_the_mode() {
        for (text, mode) in [
            ("auto", ReplacementMode::Auto),
            ("lead", ReplacementMode::Lead),
            ("off", ReplacementMode::Off),
        ] {
            let config = Config::from_toml_str(&with(&format!("[replacement]\nmode = \"{text}\"\n"))).unwrap();
            assert_eq!(config.replacement().mode(), mode, "{text}");
        }
    }

    #[test]
    fn a_mode_that_is_not_auto_lead_or_off_is_refused() {
        let error = Config::from_toml_str(&with("[replacement]\nmode = \"raed\"\n")).unwrap_err();

        assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
    }

    #[test]
    fn a_replacement_agent_that_is_not_configured_is_refused() {
        let error = Config::from_toml_str(&with("[replacement]\nagents = [\"nobody\"]\n")).unwrap_err();

        assert!(
            matches!(&error, ConfigError::ReplacementAgentUnknown { name } if name.as_str() == "nobody"),
            "{error:?}"
        );
    }

    #[test]
    fn a_replacement_agent_named_twice_is_refused() {
        let error = Config::from_toml_str(&with("[replacement]\nagents = [\"claude-2\", \"claude-2\"]\n")).unwrap_err();

        assert!(
            matches!(&error, ConfigError::ReplacementAgentTwice { name } if name.as_str() == "claude-2"),
            "{error:?}"
        );
    }

    #[test]
    fn an_investigator_may_take_a_task_over() {
        let config = Config::from_toml_str(
            "[agents.claude-1]\ncli = \"claude\"\n\n[investigators.inv-opus]\ncli = \"claude\"\n\n\
             [tiers]\nnormal = [\"claude-1\"]\nworkers = []\n\n[replacement]\nagents = [\"inv-opus\"]\n",
        )
        .unwrap();

        assert_eq!(names(config.replacement().agents()), ["inv-opus"]);
    }

    #[test]
    fn more_takeovers_than_a_task_can_have_are_refused() {
        let error = Config::from_toml_str(&with("[replacement]\nreplacements = 11\n")).unwrap_err();

        assert!(matches!(&error, ConfigError::Parse(_)), "{error:?}");
    }
}
