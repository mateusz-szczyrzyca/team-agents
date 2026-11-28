//! The decisions of the harness: who is in a team and who coordinates it ([`team`]), which item goes to which member
//! ([`assign`]), what the checks before a review find in a round ([`checks`]), what happens next in the round of one
//! item ([`round`]), how a finished run is told apart ([`outcome`]), whether a run that ends done goes back for its
//! task's plan ([`plan`]), whether the branch of a lead that waits is
//! published, gated or sent back ([`publish`]) and how long a failed step waits before it is tried again ([`retry`]).
//!
//! Every decision is a pure function of its arguments — the clock and the random source included — and returns
//! what the daemon is to do, never doing it: no file, process or network is touched here (`docs/architecture.md`).

pub mod account_candidates;
pub mod account_pause;
pub mod asleep;
pub mod assign;
pub mod busy;
pub mod checks;
pub mod commit_guard;
pub mod commit_reminder;
pub mod context;
pub mod context_reminder;
pub mod delivered;
pub mod delivery;
pub mod disclosure;
pub mod findings;
pub mod fix;
pub mod idle;
pub mod ladder;
pub mod last_step;
pub mod limit;
pub mod merge_refusal;
pub mod no_tool_call;
pub mod opinion;
pub mod outcome;
pub mod part;
pub mod plan;
pub mod pool;
pub mod publish;
pub mod refusal;
pub mod rejection;
pub mod reminders;
pub mod replacement;
pub mod restart;
pub mod retry;
pub mod reviewer;
pub mod round;
pub mod round_limit;
pub mod session_account;
pub mod slot;
pub mod split;
pub mod team;

pub mod wait;
pub mod worker_result;
pub mod worker_wait;

/// Deterministic assignment of investigators to any team activity.
pub mod investigator;

/// Bounded recovery of requests whose worker supplied no result.
pub mod worker_recovery;
