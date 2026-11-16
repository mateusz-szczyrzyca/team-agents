//! The notifier: delivers the notifications the owner asks for, so that a slow command never holds up the state.

use agent_core::history::{EventRecord, HistoryEvent, Record};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::state::Timestamp;
use agent_runtime::history::History;
use agent_runtime::notify::{Notification, Notifier};
use tokio::sync::mpsc;

use agent_runtime::process::error_chain;

/// How many notifications may wait for delivery; more are dropped and logged.
pub(super) const QUEUE: usize = 256;

/// A notification to deliver.
#[derive(Debug)]
pub(super) struct Delivery {
    /// When it happened.
    pub(super) at: Timestamp,
    /// The team of the task.
    pub(super) team: TeamId,
    /// The rounds the task works for: the history links the notification to them.
    pub(super) rounds: Vec<RoundId>,
    /// The task it is about.
    pub(super) task: TaskId,
    /// What to tell.
    pub(super) notification: Notification,
}

/// Delivers what arrives on `queue` until it is closed and empty.
///
/// A notification that cannot be rendered or delivered is logged and goes no further: it tells the human about
/// something that already happened, and the daemon does not stop for it.
pub(super) async fn run(mut queue: mpsc::Receiver<Delivery>, notifier: Notifier, history: History) {
    while let Some(delivery) = queue.recv().await {
        let rounds = delivery
            .rounds
            .iter()
            .map(RoundId::as_str)
            .collect::<Vec<_>>()
            .join(",");
        let delivery_span = tracing::error_span!(
            "notification",
            team = %delivery.team,
            rounds = %rounds,
            task = %delivery.task
        );
        match notifier.notify(delivery.at, &delivery.notification).await {
            Ok(file) => {
                tracing::info!(
                    parent: &delivery_span,
                    event = agent_core::log::LogEvent::NotificationSent.as_str(),
                    task = %delivery.task,
                    file = %file.display(),
                    "notification written"
                );
                history.record(Record::Event(Box::new(EventRecord {
                    at: delivery.at,
                    team: delivery.team.clone(),
                    rounds: delivery.rounds.clone(),
                    event: HistoryEvent::Notified {
                        task: Some(delivery.task.clone()),
                        title: delivery.notification.title().to_owned(),
                        body: delivery.notification.body().to_owned(),
                    },
                })));
            }
            Err(error) => {
                tracing::error!(
                    parent: &delivery_span,
                    event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                    task = %delivery.task,
                    error = %error_chain(&error),
                    "cannot deliver a notification"
                );
            }
        }
    }
}
