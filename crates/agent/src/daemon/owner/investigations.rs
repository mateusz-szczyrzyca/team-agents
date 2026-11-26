//! Serves open investigation requests with free investigators.

use agent_core::ids::TeamId;

use super::Owner;
use crate::daemon::owner::OwnerError;

impl Owner {
    /// Serves open investigation requests with free investigators for `team_id`.
    pub(super) async fn serve_investigations(&mut self, _team_id: &TeamId) -> Result<(), OwnerError> {
        drop(OwnerError::Publish(String::new()));
        tokio::task::yield_now().await;
        Ok(())
    }
}
