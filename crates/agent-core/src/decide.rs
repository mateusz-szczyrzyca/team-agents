//! The decisions of the harness: who is in a team and who coordinates it ([`team`]), which item goes to which member
//! ([`assign`]), what the checks before a review find in a round ([`checks`]), what happens next in the round of one
//! item ([`round`]) and how a finished run is told apart ([`outcome`]).
//!
//! Every decision is a pure function of its arguments — the clock and the random source included — and returns
//! what the daemon is to do, never doing it: no file, process or network is touched here (`docs/architecture.md`).

pub mod assign;
pub mod checks;
pub mod outcome;
pub mod round;
pub mod team;

/// How many reviews every pull request gets, each from a different investigator outside the team.
pub const REVIEWS_REQUIRED: usize = 2;
