//! The lines that open every list of `agent status`: the versions of this command and, when the daemon's differ or it
//! does not tell them, the daemon's.

use agent_core::protocol::Versions;

use super::versions::block;

/// The versions of a build, the first crate of which is `agent` 0.9.1.
fn build() -> Versions {
    Versions::new("0.9.1", "0.9.2", "0.9.3", "0.9.4")
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
    let daemon = Versions::new("0.7.1", "0.7.2", "0.7.3", "0.7.4");

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
