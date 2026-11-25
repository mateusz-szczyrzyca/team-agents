//! Whether a session is reminded that its context is near its bound: the context reached `percent` of the bound, and the
//! session was not told yet.

/// Whether the session whose context holds `context` tokens is reminded now, given its agent's `bound` in tokens (`None`
/// when the bound is off), the `percent` of the bound at which a session is reminded (`0` turns the reminder off) and
/// whether the session was `told` already.
///
/// The comparison is made in `u128`, so that no context and bound a CLI can report overflows it.
#[must_use]
pub fn context_due(context: u64, bound: Option<u64>, percent: u8, told: bool) -> bool {
    let Some(bound) = bound else {
        return false;
    };
    if percent == 0 || told {
        return false;
    }
    u128::from(context) * 100 >= u128::from(bound) * u128::from(percent)
}

#[cfg(test)]
mod tests;
