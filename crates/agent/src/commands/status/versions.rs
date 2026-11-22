//! The lines that open every list of `agent status`: the versions of the command and, when the daemon's differ or the
//! daemon does not tell them, the daemon's.

use agent_core::protocol::Versions;

/// The lines that open a list: the command's versions, then the daemon's when they differ from them or are unknown.
/// `daemon` is `None` when the daemon did not tell its versions.
#[must_use]
pub(crate) fn block(_command: &Versions, _daemon: Option<&Versions>) -> String {
    String::new()
}
