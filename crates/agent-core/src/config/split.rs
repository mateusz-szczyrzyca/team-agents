//! Policy for checking split proposals, independent of deliverable kind.

/// Limits used by the pure split approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Split {
    /// Largest estimate in the repository's unit; zero disables size checks.
    pub part_limit: u32,
    /// Send-backs allowed before asking the human; zero asks at the first fault.
    pub corrections: u32,
}
impl Default for Split {
    fn default() -> Self {
        Self {
            part_limit: 1500,
            corrections: 2,
        }
    }
}

#[cfg(test)]
mod tests;
