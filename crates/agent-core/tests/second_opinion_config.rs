//! Configuration compatibility for the dormant second-opinion protocol.
use agent_core::config::Config;

#[test]
fn second_opinions_default_on_and_can_be_disabled() {
    for (text, expected) in [
        ("", true),
        ("[review]\nfix_passes = 2", true),
        ("[review]\nallow_second_opinion_on_blocker = false", false),
    ] {
        let config = Config::from_toml_str(text).expect("valid review configuration");
        assert_eq!(config.review().allow_second_opinion_on_blocker, expected);
    }
}
