//! Required history snapshots explain automatic removal even after state disappears.
use super::super::removal::{branches_of, internal};
use super::{Owner, TeamId};
use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogRecord, LogTeamScope};
use agent_core::protocol::{Failure, FailureKind};
use agent_core::state::Timestamp;

/// Whether the snapshot precedes removal or records its refusal.
#[derive(Debug, Clone, Copy)]
pub(super) enum Outcome {
    /// Removal is about to be attempted.
    Attempt,
    /// The safety checks retained the team.
    Refused,
}
impl Owner {
    /// Disabled history is an explicit opt-out; unavailable enabled history blocks deletion.
    pub(super) async fn journal_idle(&self, id: &TeamId, outcome: Outcome, reason: &str) -> Result<(), Failure> {
        if self.history.is_disabled() {
            return Ok(());
        }
        let team = self.parts.state.team(id).ok_or_else(|| Failure {
            kind: FailureKind::Internal,
            message: "idle team disappeared".into(),
        })?;
        let outcome = match outcome {
            Outcome::Attempt => "attempt",
            Outcome::Refused => "refused",
        };
        let fields = serde_json::json!({
            "decision": "idle_removal", "outcome": outcome, "reason": reason,
            "last_activity": team.idle_watch().map(|watch| watch.last_activity),
            "items": team.items(), "rounds": team.rounds(),
            "branches": branches_of(team)?,
        });
        let fields = serde_json::from_value(fields).map_err(|e| internal("cannot capture idle context", &e))?;
        let record = LogRecord {
            at: Timestamp::now(&self.parts.clock).map_err(|e| internal("cannot timestamp idle decision", &e))?,
            module: module_path!().into(),
            event: LogEvent::DecisionTaken,
            team: Some(LogTeamScope::new(
                id.clone(),
                team.open_rounds().map(|round| round.id().clone()),
            )),
            task: None,
            message: reason.into(),
            fields,
            cut: Cut::default(),
        };
        self.history
            .persist_log(record)
            .await
            .map_err(|e| internal("cannot persist idle removal history", &e))
    }
}
