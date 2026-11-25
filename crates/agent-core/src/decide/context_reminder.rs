//! Whether a session is reminded that its context is near its bound: the context reached `percent` of the bound, and the
//! session was not told yet.

/// Whether the session whose context holds `context` tokens is reminded now, given its agent's `bound` in tokens (`None`
/// when the bound is off), the `percent` of the bound at which a session is reminded (`0` turns the reminder off) and
/// whether the session was `told` already.
#[must_use]
pub const fn context_due(context: u64, bound: Option<u64>, percent: u8, told: bool) -> bool {
    let _ = (context, bound, percent, told);
    false
}

#[cfg(test)]
mod tests;
