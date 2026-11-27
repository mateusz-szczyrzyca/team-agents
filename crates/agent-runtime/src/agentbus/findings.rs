//! Decode the bus timestamp layout using the machine zone supplied by the daemon.
use super::{AgentBus, AgentBusError, Args, BusCommand, decode_json};
use agent_core::config::RepoName;
use agent_core::decide::findings::{Finding, FindingId, FindingKind, RepoPath};
use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use serde::Deserialize;

/// Open finding as emitted by the Go bus, whose timestamp has no offset.
#[derive(Debug, Deserialize)]
struct BusFinding {
    /// Stable finding number.
    id: FindingId,
    /// Local addition time, or RFC 3339 for compatibility.
    ts: String,
    /// Agent that wrote the note.
    author: String,
    /// Task that wrote the note.
    task: TaskId,
    /// Finding category.
    kind: FindingKind,
    /// Repository paths to match.
    paths: Vec<RepoPath>,
    /// Note body.
    text: String,
}

impl AgentBus {
    /// Reads open findings, interpreting offset-free bus timestamps as UTC.
    /// Daemon callers use [`Self::findings_in`] with their machine zone.
    ///
    /// # Errors
    /// Returns [`AgentBusError`] if the command or decoding fails.
    pub async fn findings(&self, repo: &RepoName) -> Result<Vec<Finding>, AgentBusError> {
        self.findings_in(repo, Zone::Utc).await
    }

    /// Reads open findings using the bus machine's time zone.
    ///
    /// # Errors
    /// Returns [`AgentBusError`] if the command, JSON or timestamp decoding fails.
    pub async fn findings_in(&self, repo: &RepoName, zone: Zone) -> Result<Vec<Finding>, AgentBusError> {
        let command = BusCommand::Findings;
        let output = self
            .run(
                command,
                Args::new().flag("--repo", repo.as_str()).switch("--json"),
                None,
            )
            .await?;
        let rows: Vec<BusFinding> = decode_json(command, &output)?;
        rows.into_iter()
            .map(|row| {
                let added_at = zone
                    .parse_typed(&row.ts)
                    .map_err(|source| AgentBusError::FindingTime { id: row.id, source })?;
                Ok(Finding {
                    id: row.id,
                    added_at,
                    author: row.author,
                    task: row.task,
                    kind: row.kind,
                    paths: row.paths,
                    text: row.text,
                })
            })
            .collect()
    }
}
