//! The owner's side of the team's life: starting a team from a `run`, and keeping it going.
//!
//! The decisions are in `daemon/team`; this carries them out: git for the workspaces, the bus for the members' states,
//! the messages and the requests for workers, the state for the assignments, and the notification that a team is done.

#[cfg(test)]
mod tests;
