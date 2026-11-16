//! Regression coverage for publication warning identity and round selection.

use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;

use super::*;
use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{solo_state, team_id};

/// The warnings observed before history coalescing can hide duplicate emission.
#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
    fn on_event(&self, event: &tracing::Event<'_>, _context: tracing_subscriber::layer::Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut reason = Reason::default();
            event.record(&mut reason);
            self.0.lock().unwrap().push(reason.0);
        }
    }
}

/// A warning's reason as recorded through tracing's display adapter.
#[derive(Default)]
struct Reason(String);

impl tracing::field::Visit for Reason {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "reason" {
            self.0 = format!("{value:?}");
        }
    }
}

#[tokio::test]
async fn returning_to_a_previous_wait_reason_does_not_emit_it_again() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    let pulls = super::super::Pulls {
        texts: Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&dir, language)
                .await
                .unwrap(),
        ),
        checks: Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&dir, language)
                .await
                .unwrap(),
        ),
        settings: agent_runtime::github::GitHubSettings::default(),
        token: agent_core::config::Secret::try_from("test-token".to_owned()).unwrap(),
        trusted: vec![],
    };
    let team = team_id(0);
    let lead = owner.lead_of(&team).unwrap();
    let publication = Publication {
        head_branch: "agents/claude-1/finished".parse().unwrap(),
        publish: Publish::Open { items: vec![] },
    };
    let site = Site {
        pulls: &pulls,
        team: &team,
        lead: &lead,
    };
    let tip: CommitId = "1111111111111111111111111111111111111111".parse().unwrap();
    let warnings = Warnings::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(warnings.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        for reason in [
            PublicationWaitReason::NoGateRecord,
            PublicationWaitReason::GateFailed,
            PublicationWaitReason::NoGateRecord,
        ] {
            owner.log_publication_waits(site, &publication, Some(&tip), None, reason);
        }
    });
    assert_eq!(*warnings.0.lock().unwrap(), ["no_gate_record", "gate_failed"]);
}

#[test]
fn a_publication_uses_its_items_round_when_an_older_round_is_also_open() {
    let mut state = solo_state();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    let older: RoundId = "round-20261111-153001-k3x9".parse().unwrap();
    let relevant: RoundId = "round-20261111-153002-k3x9".parse().unwrap();
    team.open_round(older, Some("1.1".parse().unwrap()), crate::fixtures::at(1))
        .unwrap();
    team.open_round(relevant.clone(), Some("1.2".parse().unwrap()), crate::fixtures::at(2))
        .unwrap();
    let publication = Publication {
        head_branch: "agents/claude-1/second".parse().unwrap(),
        publish: Publish::Open {
            items: vec![Joining {
                item: "1.2".parse().unwrap(),
                branch: "agents/claude-1/second".parse().unwrap(),
            }],
        },
    };
    assert_eq!(
        publication_round(state.team(&team_id(0)).unwrap(), &publication),
        Some(relevant)
    );
}
