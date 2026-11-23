//! What a run's stream told of its context and cost: the `run_usage` table, kept outside the ledger like judgements.

use agent_core::cli::RunUsage;
use agent_core::ids::RunId;

/// One run's usage as the history keeps it. Each part of `usage` is `None` where the CLI did not tell it.
#[derive(Debug, Clone, PartialEq)]
pub struct RunUsageRecord {
    /// The run.
    pub run: RunId,
    /// What the run's stream showed.
    pub usage: RunUsage,
}
