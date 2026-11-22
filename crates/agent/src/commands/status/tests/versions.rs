//! The lines that open every list of `agent status`: the versions of this command and, when the daemon's differ or it
//! does not tell them, the daemon's.

use agent_core::protocol::Versions;

use super::versions::block;

/// The versions of a build, the first crate of which is `agent` 0.9.1.
fn build() -> Versions {
    Versions {
        agent: String::from("0.9.1"),
        agent_core: String::from("0.9.2"),
        agent_runtime: String::from("0.9.3"),
        agent_web: String::from("0.9.4"),
    }
}

#[test]
fn the_versions_of_the_command_are_one_line_with_the_crates_in_order() {
    assert_eq!(
        block(&build(), Some(&build())),
        "agent 0.9.1 \u{b7} agent-core 0.9.2 \u{b7} agent-runtime 0.9.3 \u{b7} agent-web 0.9.4\n"
    );
}

#[test]
fn a_daemon_of_the_same_versions_adds_no_line() {
    let text = block(&build(), Some(&build()));

    assert_eq!(text.lines().count(), 1, "{text}");
}

#[test]
fn a_daemon_that_differs_in_one_crate_says_its_versions_and_that_they_differ() {
    let mut daemon = build();
    daemon.agent_web = String::from("0.8.0");

    assert_eq!(
        block(&build(), Some(&daemon)),
        "agent 0.9.1 \u{b7} agent-core 0.9.2 \u{b7} agent-runtime 0.9.3 \u{b7} agent-web 0.9.4\n\
         daemon: agent 0.9.1 \u{b7} agent-core 0.9.2 \u{b7} agent-runtime 0.9.3 \u{b7} agent-web 0.8.0 \
         (differs from this command)\n"
    );
}

#[test]
fn a_daemon_that_differs_in_every_crate_is_told_in_full() {
    let daemon = Versions {
        agent: String::from("0.7.1"),
        agent_core: String::from("0.7.2"),
        agent_runtime: String::from("0.7.3"),
        agent_web: String::from("0.7.4"),
    };

    assert_eq!(
        block(&build(), Some(&daemon)),
        "agent 0.9.1 \u{b7} agent-core 0.9.2 \u{b7} agent-runtime 0.9.3 \u{b7} agent-web 0.9.4\n\
         daemon: agent 0.7.1 \u{b7} agent-core 0.7.2 \u{b7} agent-runtime 0.7.3 \u{b7} agent-web 0.7.4 \
         (differs from this command)\n"
    );
}

#[test]
fn a_daemon_that_does_not_tell_its_versions_is_version_unknown() {
    assert_eq!(
        block(&build(), None),
        "agent 0.9.1 \u{b7} agent-core 0.9.2 \u{b7} agent-runtime 0.9.3 \u{b7} agent-web 0.9.4\n\
         daemon: version unknown\n"
    );
}

#[test]
fn versions_table_covers_all_equal_one_different_and_daemon_equal_different_or_unknown() {
    let equal = Versions {
        agent: String::from("0.9.0"),
        agent_core: String::from("0.9.0"),
        agent_runtime: String::from("0.9.0"),
        agent_web: String::from("0.9.0"),
    };
    let one_diff = Versions {
        agent: String::from("0.9.0"),
        agent_core: String::from("0.9.1"),
        agent_runtime: String::from("0.9.0"),
        agent_web: String::from("0.9.0"),
    };
    let other_daemon = Versions {
        agent: String::from("0.8.0"),
        agent_core: String::from("0.8.0"),
        agent_runtime: String::from("0.8.0"),
        agent_web: String::from("0.8.0"),
    };
    let cases = [
        (
            "all equal and daemon equal",
            &equal,
            Some(&equal),
            "agent 0.9.0 \u{b7} agent-core 0.9.0 \u{b7} agent-runtime 0.9.0 \u{b7} agent-web 0.9.0\n",
        ),
        (
            "one crate different and daemon equal",
            &one_diff,
            Some(&one_diff),
            "agent 0.9.0 \u{b7} agent-core 0.9.1 \u{b7} agent-runtime 0.9.0 \u{b7} agent-web 0.9.0\n",
        ),
        (
            "daemon different",
            &equal,
            Some(&other_daemon),
            "agent 0.9.0 \u{b7} agent-core 0.9.0 \u{b7} agent-runtime 0.9.0 \u{b7} agent-web 0.9.0\n\
             daemon: agent 0.8.0 \u{b7} agent-core 0.8.0 \u{b7} agent-runtime 0.8.0 \u{b7} agent-web 0.8.0 (differs from this command)\n",
        ),
        (
            "daemon unknown",
            &equal,
            None,
            "agent 0.9.0 \u{b7} agent-core 0.9.0 \u{b7} agent-runtime 0.9.0 \u{b7} agent-web 0.9.0\n\
             daemon: version unknown\n",
        ),
    ];

    for (name, command, daemon, expected) in cases {
        assert_eq!(block(command, daemon), expected, "{name}");
    }
}
