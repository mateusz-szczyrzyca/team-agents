//! A round whose task waits to be started is no defect: the owner writes no error for it, and a round with nothing
//! left to do still is one.

use std::fmt;
use std::sync::{Arc, Mutex};

use agent_core::ids::TeamId;
use agent_core::log::LogEvent;
use agent_core::state::{ItemPhase, RunEnd, State};
use agent_core::status::{RunOutcome, TaskStatus};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

use super::super::fixture::Daemon;
use crate::fixtures::{at, task, team_id, team_state};

/// The `event` field of each log record of one look of the owner.
#[derive(Default, Clone)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Layer<Registry> for Events {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Registry>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.event);
    }
}

/// The fields of a log record that this test reads.
#[derive(Default)]
struct Fields {
    /// The `event` field.
    event: String,
}

impl tracing::field::Visit for Fields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_debug(field, &value);
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        if field.name() == "event" {
            self.event = format!("{value:?}").trim_matches('"').to_owned();
        }
    }
}

/// An owner of a daemon home of `state` with the token of a GitHub account: it has its pull requests, and a round's
/// lead that runs the item is listed among its members.
async fn owner_running_item(state: &State) -> crate::daemon::owner::Owner {
    let daemon = home_with_token(state);
    let (owner, _sinks) = daemon.bare_owner().await;
    let pulls = pulls_of(&owner).await.expect("the fixture home has a GitHub token");
    owner.with_pulls(pulls)
}

/// A daemon home with `state` and the token of a GitHub account, so that the owner has its autopilot.
fn home_with_token(state: &State) -> Daemon {
    let daemon = Daemon::new(state, None);
    let secrets = daemon.home.root().join("secrets.toml");
    std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
    std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    daemon
}

/// The round of item 1.1 of the sample team, whose lead ended its run as `end` and is `status`.
fn lead_round(end: RunEnd, status: TaskStatus) -> (State, TeamId) {
    let mut state = team_state();
    let team = team_id(0);
    {
        let mut sample = state.team_mut(&team).unwrap();
        sample
            .open_round(
                "round-20261111-153101-q1w2".parse().unwrap(),
                Some("1.1".parse().unwrap()),
                at(1),
            )
            .unwrap();
        sample
            .advance_item(
                "1.1".parse().unwrap(),
                ItemPhase::InProgress {
                    assignee: "claude-1".parse().unwrap(),
                    branch: "agents/claude-1/1.1".parse().unwrap(),
                },
            )
            .unwrap();
        let (_, mut member) = sample.find_mut(&task(1)).unwrap();
        // A run that starts and ends in the round, as the lead's does when it hand-overs.
        member.begin_run(crate::fixtures::run(1), at(1), None).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(2), end).unwrap();
        if status == TaskStatus::Done {
            // A task ends done after a run of it, so the run that ends it comes first.
            member.transition(TaskStatus::Running).unwrap();
            member
                .end_run(
                    crate::fixtures::at(3),
                    RunEnd::Reported {
                        outcome: RunOutcome::Done,
                    },
                )
                .unwrap();
        }
        if member.status() != status {
            member.transition(status).unwrap();
        }
    }
    (state, team)
}

/// The events one check for lost obligations of the teams of `state` logs.
async fn events_of_a_look(state: &State, team: &TeamId) -> Vec<String> {
    let mut owner = owner_running_item(state).await;
    let events = Events::default();
    let subscriber = tracing_subscriber::registry().with(events.clone());
    tracing::subscriber::with_default(subscriber, || owner.observe_round_waits(team));
    events.0.lock().unwrap().clone()
}

// The round of a part opened right after a merge is idle only because its lead's run is queued: the owner reports no
// defect for it, and writes no daemon_task_failed error.
#[tokio::test]
async fn a_round_whose_run_is_queued_writes_no_error() {
    let (state, team) = lead_round(
        RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
        TaskStatus::Queued,
    );

    let logged = events_of_a_look(&state, &team).await;

    assert!(
        !logged.contains(&LogEvent::DaemonTaskFailed.as_str().to_owned()),
        "{logged:?}"
    );
}

/// The autopilot of `owner`, with the real texts.
async fn pulls_of(owner: &crate::daemon::owner::Owner) -> Option<crate::daemon::owner::Pulls> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    crate::daemon::owner::Pulls::new(
        Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        &owner.parts.settings,
    )
    .unwrap()
}
