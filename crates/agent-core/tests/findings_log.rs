//! Findings have stable, typed structured log names and levels.
use agent_core::log::{LogEvent, LogLevel};
#[test]
fn findings_log_names_have_the_required_levels() {
    for (name, level) in [
        ("findings_shown", LogLevel::Info),
        ("findings_unavailable", LogLevel::Warn),
    ] {
        assert_eq!(
            LogEvent::ALL
                .iter()
                .find(|event| event.as_str() == name)
                .map(|event| event.level()),
            Some(level),
            "{name}"
        );
    }
}
