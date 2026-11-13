//! The `[github]` section of `agents.toml` through the public API: the accounts that are trusted and the address of
//! the GitHub API, which only a GitHub Enterprise installation or a trial against a fake server changes.

#[cfg(test)]
mod config_github {
    use agent_core::config::{ApiUrl, Config, ConfigError, InvalidValue, ValueRule};

    /// A configuration with one agent, two investigators and one repository, followed by `extra`.
    fn config_with(extra: &str) -> String {
        format!(
            "[agents.claude-1]\ncli = \"claude\"\n[investigators.inv-1]\ncli = \"claude\"\n\
             [investigators.inv-2]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n\
             [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n{extra}"
        )
    }

    fn refused(text: &str) -> InvalidValue {
        text.parse::<ApiUrl>().unwrap_err()
    }

    #[test]
    fn the_api_is_the_public_one_when_the_configuration_names_none() {
        let config = Config::from_toml_str(&config_with("")).unwrap();

        assert_eq!(config.github().api_base, None);
    }

    #[test]
    fn the_configuration_names_the_api_of_an_enterprise_installation() {
        let config = Config::from_toml_str(&config_with(
            "[github]\napi_base = \"https://github.example.com/api/v3\"\n",
        ))
        .unwrap();

        assert_eq!(
            config.github().api_base.as_ref().map(ApiUrl::as_str),
            Some("https://github.example.com/api/v3")
        );
    }

    #[test]
    fn the_api_of_a_trial_may_be_a_loopback_address_over_http() {
        let url: ApiUrl = "http://127.0.0.1:8080".parse().unwrap();

        assert_eq!(url.as_str(), "http://127.0.0.1:8080");
    }

    #[test]
    fn an_api_url_that_is_not_http_is_refused_with_the_rule_it_breaks() {
        for text in [
            "ftp://github.example.com",
            "github.example.com",
            "file:///tmp/x",
            "//host/x",
        ] {
            assert_eq!(refused(text).rule, ValueRule::NotHttpUrl, "{text}");
        }
    }

    #[test]
    fn an_empty_api_url_is_refused() {
        assert_eq!(refused("").rule, ValueRule::Empty);
    }

    #[test]
    fn an_api_url_with_whitespace_or_a_control_character_is_refused() {
        assert_eq!(
            refused("https://github.example.com/a b").rule,
            ValueRule::ForbiddenChar(' ')
        );
        assert_eq!(
            refused("https://github.example.com/\u{7}").rule,
            ValueRule::ForbiddenChar('\u{7}')
        );
    }

    #[test]
    fn an_api_url_that_is_too_long_is_refused() {
        let long = format!("https://github.example.com/{}", "a".repeat(2100));

        assert!(matches!(refused(&long).rule, ValueRule::TooLong { max: 2048 }));
    }

    #[test]
    fn an_api_url_with_credentials_is_refused_without_repeating_them() {
        let error = refused("https://octo:ghp_secret_token@github.example.com");

        assert_eq!(error.rule, ValueRule::ForbiddenChar('@'));
        assert!(!format!("{error}").contains("ghp_secret_token"), "{error}");
        assert!(!error.value.contains("ghp_secret_token"), "{error:?}");
    }

    #[test]
    fn a_configuration_with_a_bad_api_url_is_refused() {
        let text = config_with("[github]\napi_base = \"ftp://github.example.com\"\n");

        assert!(matches!(Config::from_toml_str(&text), Err(ConfigError::Parse(_))));
    }
}
