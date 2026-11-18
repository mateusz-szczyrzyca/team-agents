//! The decisions of the harness: who is in a team and who coordinates it ([`team`]), which item goes to which member
//! ([`assign`]), what the checks before a review find in a round ([`checks`]), what happens next in the round of one
//! item ([`round`]), how a finished run is told apart ([`outcome`]) and whether the branch of a lead that waits is
//! published, gated or sent back ([`publish`]).
//!
//! Every decision is a pure function of its arguments — the clock and the random source included — and returns
//! what the daemon is to do, never doing it: no file, process or network is touched here (`docs/architecture.md`).

pub mod asleep;
pub mod assign;
pub mod checks;
pub mod disclosure;
pub mod outcome;
pub mod publish;
pub mod reviewer;
pub mod round;
pub mod team;
