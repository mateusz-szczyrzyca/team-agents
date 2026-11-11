//! The pure core of the agent harness: identifiers, kinds, statuses and, later, the configuration, the state and the
//! decisions (see `docs/architecture.md`).
//!
//! Nothing here does I/O: the clock and the random source are injected ([`ids::Clock`], [`rand::Rng`]), so every
//! function is deterministic under test.
#![forbid(unsafe_code)]
