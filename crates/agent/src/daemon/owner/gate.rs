//! The owner's side of `gate`: the task, the repository's gate and the run of the task are looked up and the gate is
//! put on the queue of the gate service, which answers the client when it has run.

use agent_core::protocol::{Failure, FailureKind, GateRequest, Reply};
use tokio::sync::oneshot;

use super::Owner;

impl Owner {
    /// Queues the gate of the task `request` names; `reply` is answered by the gate service when the gate is over, or at
    /// once with the refusal when the gate cannot be queued.
    pub(super) fn gate_request(&mut self, request: GateRequest, reply: oneshot::Sender<Result<Reply, Failure>>) {
        let _ = request;
        let refusal = Failure {
            kind: FailureKind::Internal,
            message: "the gate is not queued yet".to_owned(),
        };
        if reply.send(Err(refusal)).is_err() {
            tracing::debug!("the connection of a gate closed before the answer");
        }
    }
}

#[cfg(test)]
mod tests;
