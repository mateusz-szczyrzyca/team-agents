//! The systemd user unit of the daemon: the lines the daemon's behaviour depends on.

#[cfg(test)]
mod systemd_unit {
    use std::collections::BTreeMap;
    use std::path::Path;

    /// The unit's settings by section and key, the last value of a key winning.
    fn unit() -> BTreeMap<(String, String), String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/systemd/agent-daemon.service");
        let text = std::fs::read_to_string(path).unwrap();
        let mut section = String::new();
        let mut settings = BTreeMap::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
            if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
                section = name.to_owned();
            } else if let Some((key, value)) = line.split_once('=') {
                settings.insert((section.clone(), key.trim().to_owned()), value.trim().to_owned());
            }
        }
        settings
    }

    fn setting(section: &str, key: &str) -> String {
        unit()
            .get(&(section.to_owned(), key.to_owned()))
            .cloned()
            .unwrap_or_else(|| panic!("the unit has no {key} in [{section}]"))
    }

    #[test]
    fn the_unit_starts_the_daemon_command_of_the_agent_binary() {
        assert!(setting("Service", "ExecStart").ends_with("/agent daemon"), "{}", setting("Service", "ExecStart"));
    }

    #[test]
    fn the_unit_names_the_home_the_daemon_owns() {
        assert!(setting("Service", "Environment").starts_with("AGENTS_HOME=%h/"));
    }

    #[test]
    fn a_stop_sends_sigterm_to_the_daemon_alone_and_waits_for_it_to_stop_its_runs() {
        assert_eq!(setting("Service", "KillMode"), "mixed");
        let timeout = setting("Service", "TimeoutStopSec");
        assert!(timeout.parse::<u32>().unwrap() >= 30, "{timeout}");
    }

    #[test]
    fn the_unit_restarts_a_daemon_that_failed_but_not_one_that_was_stopped() {
        assert_eq!(setting("Service", "Restart"), "on-failure");
    }

    #[test]
    fn the_unit_is_wanted_by_the_default_target_of_the_user_session() {
        assert_eq!(setting("Install", "WantedBy"), "default.target");
    }
}
