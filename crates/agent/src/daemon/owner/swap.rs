//! Human-directed task swaps.

use super::{Owner, OwnerError};
use agent_core::protocol::{Client, Failure, FailureKind, Reply, SwapRequest};

impl Owner {
    /// Handles a request to change a task holder.
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
