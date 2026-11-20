//! The decisions of the harness: who is in a team and who coordinates it ([`team`]), which item goes to which member
//! ([`assign`]), what the checks before a review find in a round ([`checks`]), what happens next in the round of one
//! item ([`round`]), how a finished run is told apart ([`outcome`]), whether a run that ends done goes back for its
//! task's plan ([`plan`]), whether the branch of a lead that waits is
//! published, gated or sent back ([`publish`]) and how long a failed step waits before it is tried again ([`retry`]).
//!
//! Every decision is a pure function of its arguments — the clock and the random source included — and returns
//! what the daemon is to do, never doing it: no file, process or network is touched here (`docs/architecture.md`).

pub mod asleep;
pub mod assign;
pub mod checks;
pub mod delivery;
pub mod disclosure;
pub mod fix;
pub mod ladder;
pub mod opinion;
pub mod outcome;
pub mod plan;
pub mod publish;
pub mod retry;
pub mod reviewer;
pub mod round;
pub mod split;
pub mod team;

pub mod wait;

/// Deterministic assignment of investigators to any team activity.
pub mod investigator;
