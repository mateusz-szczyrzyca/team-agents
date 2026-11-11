//! The pure core of the agent harness: identifiers ([`ids`]), kinds ([`kinds`]), statuses ([`status`]), the
//! configuration ([`config`]), the task list ([`task_list`]) and, later, the state and the decisions (see
//! `docs/architecture.md`).
//!
//! Nothing here does I/O: the clock and the random source are passed in ([`ids::Clock`], [`rand::Rng`]), so every
//! function is deterministic under test.
#![forbid(unsafe_code)]

pub mod config;
pub mod ids;
pub mod kinds;
pub mod questions;
pub mod status;
pub mod task_list;
