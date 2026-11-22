//! The library part of `agent-web`: what the `agent` command needs of the dashboard, its version, which `agent status`
//! shows with the versions of the other crates.

/// The version of this crate, taken from its manifest at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
