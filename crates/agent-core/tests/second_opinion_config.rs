//! Configuration compatibility for the dormant second-opinion protocol.
use agent_core::config::Config;
use agent_core::review_response::opinion::Permission;

#[test]
fn second_opinions_default_on_and_can_be_disabled() {
    for (text, expected) in [
        ("", true),
        ("[review]\nfix_passes = 2", true),
        ("[review]\nallow_second_opinion_on_blocker = false", false),
    ] {
        let config = Config::from_toml_str(text).expect("valid review configuration");
        assert_eq!(config.review().second_opinion() == Permission::Enabled, expected);
    }
}

#[test]
fn the_review_section_gives_the_second_opinion_permission() {
    for (text, expected) in [
        ("", Permission::Enabled),
        ("[review]\nfix_passes = 2", Permission::Enabled),
        ("[review]\nallow_second_opinion_on_blocker = true", Permission::Enabled),
        (
            "[review]\nallow_second_opinion_on_blocker = false",
            Permission::Disabled,
        ),
    ] {
        let config = Config::from_toml_str(text).expect("valid review configuration");
        assert_eq!(config.review().second_opinion(), expected, "{text}");
    }
}
