//! The lines that open every list of `agent status`: the versions of the command and, when the daemon's differ or the
//! daemon does not tell them, the daemon's.

use agent_core::protocol::Versions;

/// The lines that open a list: the command's versions, then the daemon's when they differ from them or are unknown.
/// `daemon` is `None` when the daemon did not tell its versions.
#[must_use]
pub(crate) fn block(command: &Versions, daemon: Option<&Versions>) -> String {
    let mut lines = vec![line(command)];
    match daemon {
        None => lines.push(String::from("daemon: version unknown")),
        Some(versions) if versions == command => {}
        Some(versions) => lines.push(format!("daemon: {} (differs from this command)", line(versions))),
    }
    lines.iter().fold(String::new(), |mut text, line| {
        text.push_str(line);
        text.push('\n');
        text
    })
}

/// The versions of one build as one line: each crate's name and version, joined with a middle dot.
fn line(versions: &Versions) -> String {
    format!(
        "agent {} · agent-core {} · agent-runtime {} · agent-web {}",
        versions.agent, versions.agent_core, versions.agent_runtime, versions.agent_web
    )
}
