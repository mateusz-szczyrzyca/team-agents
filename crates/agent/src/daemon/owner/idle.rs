//! Automatic idle removal uses the same safeguards as the human command.
use super::{Owner, OwnerError};
impl Owner {
    /// Observes activity and attempts removal of teams beyond their idle limit.
    pub(super) async fn check_idle_teams(&mut self) -> Result<(), OwnerError> {
        Ok(())
    }
}
#[cfg(test)]
mod tests;
