//! The versions of this build: what `agent status` shows of the command and what the daemon tells of itself.

use agent_core::protocol::Versions;

/// The versions of this build, each of its four crates as its manifest gave it at build time.
#[must_use]
pub(crate) fn of_this_build() -> Versions {
    Versions::new(
        env!("CARGO_PKG_VERSION"),
        agent_core::VERSION,
        agent_runtime::VERSION,
        agent_web::VERSION,
    )
}
