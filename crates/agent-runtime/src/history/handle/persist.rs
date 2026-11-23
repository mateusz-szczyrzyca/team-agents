//! Acknowledged log writes for decisions that must survive before destructive work.
use super::{History, Job, QueueOutcome};
use agent_core::log::LogRecord;

/// Why a required history entry could not be persisted.
#[derive(Debug, thiserror::Error)]
pub enum PersistLogError {
    /// No writer is available, including explicitly disabled history.
    #[error("history writer is unavailable")]
    Unavailable,
    /// The database refused the entry.
    #[error(transparent)]
    Write(#[from] crate::history::WriteError),
    /// An identity contains a configured credential and cannot be stored safely.
    #[error("history entry contains a credential in its identity")]
    SecretIdentity,
    /// The blocking enqueue task failed.
    #[error("history enqueue task failed")]
    Task(#[from] tokio::task::JoinError),
    /// The writer stopped before acknowledging the entry.
    #[error("history acknowledgement was dropped")]
    Acknowledged(#[from] tokio::sync::oneshot::error::RecvError),
}
impl History {
    /// Persists a log entry, returning only after its transaction completes.
    ///
    /// # Errors
    /// Returns [`PersistLogError`] if no writer exists, redaction refuses an identity,
    /// the write fails, or the writer stops before acknowledging it.
    pub async fn persist_log(&self, record: LogRecord) -> Result<(), PersistLogError> {
        let queue = self.queue.clone().ok_or(PersistLogError::Unavailable)?;
        let (ack, rx) = tokio::sync::oneshot::channel();
        let outcome = tokio::task::spawn_blocking(move || match queue.send(Job::PersistLog { record, ack }) {
            Ok(()) => QueueOutcome::Enqueued,
            Err(_) => QueueOutcome::Closed,
        })
        .await?;
        if outcome == QueueOutcome::Closed {
            return Err(PersistLogError::Unavailable);
        }
        rx.await?
    }
}
#[cfg(test)]
mod tests;
