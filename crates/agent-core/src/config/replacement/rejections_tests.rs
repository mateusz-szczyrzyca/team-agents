//! `[replacement] rejections`: how many times the lead may return an item before the agent that holds it is replaced.

#[cfg(test)]
mod rejections {
    use crate::config::Config;

    /// The smallest configuration: one normal agent, which is all a task needs.
    fn one_agent() -> &'static str {
        "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n"
    }

    /// The configuration of [`one_agent`] with `table` after it.
    fn with(table: &str) -> String {
        format!("{}{table}", one_agent())
    }

    #[test]
    fn a_file_without_replacement_replaces_the_agent_after_five_rejections() {
        let config = Config::from_toml_str(one_agent()).unwrap();

        assert_eq!(config.replacement().rejections().get(), 5);
    }

    #[test]
    fn the_replacement_table_says_after_how_many_rejections_the_agent_is_replaced() {
        for (text, expected) in [(0_u32, 0_u32), (1, 1), (5, 5), (10, 10)] {
            let config = Config::from_toml_str(&with(&format!("[replacement]\nrejections = {text}\n"))).unwrap();
            assert_eq!(config.replacement().rejections().get(), expected, "rejections = {text}");
        }
    }

    #[test]
    fn more_rejections_than_a_lead_can_make_are_refused() {
        let error = Config::from_toml_str(&with("[replacement]\nrejections = 11\n")).unwrap_err();

        assert!(
            matches!(&error, ConfigError::Parse(parse) if parse.span().is_some() && parse.message().contains("11")),
            "{error:?}"
        );
        let negative = Config::from_toml_str(&with("[replacement]\nrejections = -1\n")).unwrap_err();
        assert!(matches!(negative, ConfigError::Parse(_)), "{negative:?}");
    }

    use crate::config::ConfigError;
}
