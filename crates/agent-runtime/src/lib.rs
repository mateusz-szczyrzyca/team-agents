//! Effects of the agent harness: the state store, processes, git, the GitHub API, agentbus, prompts and
//! notifications (see `docs/architecture.md`).
//!
//! Everything here does I/O on behalf of the daemon; the decisions it carries out live in `agent-core`. The crate is
//! empty until the runtime items of the plan fill it.
#![forbid(unsafe_code)]
