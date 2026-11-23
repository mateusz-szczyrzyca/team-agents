//! Whether the next run of a task resumes its session or starts a new one, by the size of the context its last run
//! reached. A session above its agent's bound is never resumed: the next run begins a session of the harness's own,
//! briefed from the checkpoint. A size the CLI did not tell, or a bound that is off, resumes the session as before.
//!
//! [`starts_anew`] is the whole decision. It touches nothing: the owner reads the size and the bound and records the
//! renewal.

#[cfg(test)]
mod tests;

/// Whether a session whose last run reached `last_context` tokens is started anew, given the agent's `bound` in tokens
/// (`None` when the bound is off). A context equal to the bound is still resumed: only a context above it is too big.
#[must_use]
pub const fn starts_anew(last_context: Option<u64>, bound: Option<u64>) -> bool {
    match (last_context, bound) {
        (Some(last), Some(bound)) => last > bound,
        (None, _) | (_, None) => false,
    }
}
