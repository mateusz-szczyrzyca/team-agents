//! `Owner::swap_request`: changes the agent holding a task on the human's word.

use agent_core::protocol::{Client, Failure, FailureKind, Reply, SwapRequest};

use super::{Owner, OwnerError};

impl Owner {
    /// Handles a swap request from a client: ends a run that goes on, hands the task over, and returns the reply.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn swap_request(
        &mut self,
        _client: &Client,
        _request: SwapRequest,
    ) -> Result<Result<Reply, Failure>, OwnerError> {
        Ok(Err(Failure {
            kind: FailureKind::Internal,
            message: "swap stub".to_owned(),
        }))
    }
}
