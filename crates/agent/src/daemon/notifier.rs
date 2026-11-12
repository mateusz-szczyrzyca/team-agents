//! The notifier: delivers the notifications the owner asks for, so that a slow command never holds up the state.

use std::sync::Arc;

use agent_core::ids::TaskId;
use agent_core::state::Timestamp;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::notify::Notifier;
use tokio::sync::mpsc;

use super::notice::Notice;
use crate::chain::chain;

/// How many notifications may wait for delivery; more are dropped and logged.
pub(super) const QUEUE: usize = 256;

/// A notification to deliver.
#[derive(Debug)]
pub(super) struct Delivery {
    /// When it happened.
    pub(super) at: Timestamp,
    /// The agent it is about.
    pub(super) agent: String,
    /// The task it is about.
    pub(super) task: TaskId,
    /// What to tell.
    pub(super) notice: Notice,
}

/// Delivers what arrives on `queue` until it is closed and empty.
///
/// A notification that cannot be rendered or delivered is logged and goes no further: it tells the human about
/// something that already happened, and the daemon does not stop for it.
pub(super) async fn run(mut queue: mpsc::Receiver<Delivery>, texts: Arc<HarnessPrompts>, notifier: Notifier) {
    while let Some(delivery) = queue.recv().await {
        let notification = match delivery.notice.render(&texts, &delivery.agent, &delivery.task) {
            Ok(notification) => notification,
            Err(error) => {
                tracing::error!(task = %delivery.task, error = %chain(&error), "cannot render a notification");
                continue;
            }
        };
        match notifier.notify(delivery.at, &notification).await {
            Ok(file) => tracing::info!(task = %delivery.task, file = %file.display(), "notification written"),
            Err(error) => {
                tracing::error!(task = %delivery.task, error = %chain(&error), "cannot deliver a notification");
            }
        }
    }
}
