//! The daemon's side of the history: opening it from `[history]` of the configuration, and what the owner records in it.
//!
//! The history never stops or delays the work (`docs/history.md`): when the database cannot be opened the daemon says so
//! once and runs without it, and every record that is handed in after that is counted as lost.

use std::sync::Arc;

use agent_core::config::Config;
use agent_core::decide::team::TeamRequest;
use agent_core::history::{
    ExplainError, HistoryHealth, HistoryState, ItemText, RoundOpening, RoundSubject, TeamStart, explain_plan,
};
use agent_core::ids::BranchName;
use agent_core::kinds::Role;
use agent_core::protocol::RunRequest;
use agent_core::state::{Round, Team};
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Reporter, Setup, TracingReporter, Writer};
use agent_runtime::process::error_chain;
use agent_runtime::settings::Settings;
use agent_runtime::store::Home;

/// The history of a daemon, and the thread that writes it.
pub(super) struct Opened {
    /// The handle everything records through.
    pub(super) history: History,
    /// The writer, to close when the daemon ends; `None` when nothing is written.
    pub(super) writer: Option<Writer>,
}

/// Opens the history of `home` as `settings` say: off, written, or unavailable (the database cannot be used).
pub(super) async fn open(settings: &Settings, home: &Home) -> Opened {
    let config = settings.config.history();
    if !config.enabled {
        return Opened {
            history: History::disabled(),
            writer: None,
        };
    }
    let reporter: Arc<dyn Reporter> = Arc::new(TracingReporter);
    let path = home.history_file();
    let limit = config.max_text_bytes;
    // Opening makes the file, switches it to write-ahead logging and migrates it: blocking work, off the runtime.
    let opened = tokio::task::spawn_blocking(move || HistoryDb::open(&path, limit)).await;
    let database = match opened {
        Ok(Ok(database)) => database,
        Ok(Err(error)) => {
            return Opened {
                history: History::unavailable(&error, reporter),
                writer: None,
            };
        }
        Err(error) => {
            tracing::error!(error = %error_chain(&error), "the task that opens the history failed: the daemon runs without it");
            return Opened {
                history: History::disabled(),
                writer: None,
            };
        }
    };
    let setup = Setup {
        reporter: Arc::clone(&reporter),
        capacity: DEFAULT_QUEUE,
    };
    match History::start(database, setup) {
        Ok(started) => Opened {
            history: started.history,
            writer: Some(started.writer),
        },
        Err(error) => {
            tracing::error!(error = %error_chain(&error), "the writer of the history cannot be started: the daemon runs without it");
            Opened {
                history: History::disabled(),
                writer: None,
            }
        }
    }
}

/// Closes the writer of the history when the daemon ends, once everything that records has ended: what is queued is written
/// and the thread is joined. A history that does not close cleanly is told, and the daemon ends all the same.
pub(super) async fn close(writer: Option<Writer>) {
    let Some(writer) = writer else {
        return;
    };
    match tokio::task::spawn_blocking(move || writer.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::error!(error = %error_chain(&error), "the history writer did not close cleanly: records may be missing");
        }
        Err(error) => {
            tracing::error!(error = %error_chain(&error), "the task that closes the history writer failed: records may be missing");
        }
    }
}

/// The record of the start of the team `team`, asked for by `request`, whose branches start from `base`.
///
/// # Errors
/// [`ExplainError`] when the people of the team do not come from the request: a mistake of the daemon.
pub(super) fn team_start(
    request: &RunRequest,
    team: &Team,
    base: &BranchName,
    config: &Config,
) -> Result<TeamStart, ExplainError> {
    let crew: Vec<(Role, String)> = team
        .crew()
        .roster()
        .into_iter()
        .map(|(role, member)| (role, member.agent().name().to_owned()))
        .collect();
    let plan = explain_plan(
        &TeamRequest {
            agents: &request.agents,
            flag_coordinator: request.coordinator.as_ref(),
            listed_coordinator: request.task_list.list().coordinator(),
        },
        config,
        &crew,
    )?;
    Ok(TeamStart {
        team: team.id().clone(),
        at: team.created_at(),
        repo: team.repo().to_string(),
        base_branch: base.clone(),
        merge: team.merge(),
        task_list: team.task_list().text().to_owned(),
        plan,
    })
}

/// The record of the opening of `round` of `team`: its item in full, or the brief of a list that has none; `None` when the
/// item is not in the task list, which the state does not allow.
pub(super) fn round_opening(team: &Team, round: &Round) -> Option<RoundOpening> {
    let list = team.task_list().list();
    let subject = match round.item() {
        Some(id) => {
            let item = list.item(id)?;
            RoundSubject::Item(ItemText {
                item: id,
                title: item.title.clone(),
                description: item.description.clone(),
                expected_result: item.expected_result.clone(),
                checks: item.checks.clone(),
            })
        }
        None => RoundSubject::Brief(list.brief().to_owned()),
    };
    Some(RoundOpening {
        round: round.id().clone(),
        team: team.id().clone(),
        at: round.opened_at(),
        subject,
    })
}

/// What the history is to say of its health in an answer to `status`: nothing when it is written and whole or off, the
/// health when it lost records or cannot be written.
pub(super) fn health_to_tell(history: &History) -> Option<HistoryHealth> {
    let health = history.health();
    match (&health.state, health.lost) {
        (HistoryState::Disabled, _) | (HistoryState::Writing, 0) => None,
        (HistoryState::Writing | HistoryState::Unavailable { .. }, _) => Some(health),
    }
}

#[cfg(test)]
mod tests;
