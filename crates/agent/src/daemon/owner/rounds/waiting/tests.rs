//! A lost scheduling obligation is an error once, never a repeated idle-tick message.

use std::sync::{Arc, Mutex};

use agent_core::decide::wait::{Obligation, Waiting};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::solo_state;

/// Error levels emitted while observing a round.
#[derive(Clone, Default)]
struct Errors(Arc<Mutex<Vec<tracing::Level>>>);

impl Layer<Registry> for Errors {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Registry>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            self.0.lock().unwrap().push(*event.metadata().level());
        }
    }
}

#[tokio::test]
async fn an_unowned_idle_round_is_logged_as_an_error_once() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let round = "round-20261119-090000-test".parse().unwrap();
    let errors = Errors::default();
    let subscriber = tracing_subscriber::registry().with(errors.clone());
    tracing::subscriber::with_default(subscriber, || {
        owner.observe_wait(&round, None);
        owner.observe_wait(&round, Some(Waiting::For(Obligation::Human)));
        owner.observe_wait(&round, Some(Waiting::Defect));
        owner.observe_wait(&round, Some(Waiting::Defect));
        owner.observe_wait(&round, None);
        owner.observe_wait(&round, Some(Waiting::Defect));
    });
    assert_eq!(*errors.0.lock().unwrap(), vec![tracing::Level::ERROR]);
}
