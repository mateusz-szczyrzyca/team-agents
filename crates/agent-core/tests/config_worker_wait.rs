//! `wait_seconds` under `[workers]`, through the public API: how long a request waits for a worker before it is returned.

#[cfg(test)]
mod config_worker_wait {
    use agent_core::config::Config;

    /// A configuration with one agent, followed by `extra`.
    fn config_with(extra: &str) -> String {
        format!("[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n{extra}")
    }

    fn wait_of(text: &str) -> u64 {
        let config = Config::from_toml_str(text).unwrap_or_else(|error| panic!("{error:?}"));
        config.workers().wait_seconds()
    }

    // Rule: a configuration without the key, or without `[workers]`, waits ten minutes.
    #[test]
    fn a_configuration_without_the_key_waits_600_seconds() {
        assert_eq!(wait_of(&config_with("")), 600);
        assert_eq!(wait_of(&config_with("[workers]\ntries = 1\n")), 600);
    }

    // Rule: the key is read as given, `0` included (no limit).
    #[test]
    fn the_key_is_read_as_given() {
        assert_eq!(wait_of(&config_with("[workers]\nwait_seconds = 90\n")), 90);
        assert_eq!(wait_of(&config_with("[workers]\nwait_seconds = 0\n")), 0);
    }

    // Rule: the other keys of the table keep their meaning.
    #[test]
    fn the_other_keys_are_unchanged() {
        let config =
            Config::from_toml_str(&config_with("[workers]\nwait_seconds = 5\ntries = 1\nseconds = 7\n")).unwrap();
        assert_eq!(config.workers().tries(), 1);
        assert_eq!(config.workers().seconds(), 7);
    }

    // Rule: the example configuration names the key with its default.
    #[test]
    fn the_example_names_the_key_with_its_default() {
        let example = include_str!("../../../config/agents.example.toml");
        assert!(
            example.contains("wait_seconds = 600"),
            "the example has no `wait_seconds = 600` line"
        );
    }
}
