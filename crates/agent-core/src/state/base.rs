//! Evidence retained when the human permits starting on unpublished base commits.

use super::CommitId;
use crate::ids::BranchName;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

/// The base commits missing from origin when a team was explicitly allowed to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnpushedBase {
    /// The local base branch compared with origin.
    pub branch: BranchName,
    /// Number of local commits not reachable from origin's base; always positive.
    pub count: NonZeroU64,
    /// The first missing commit in reverse topological order (parents before children).
    pub first: CommitId,
    /// The first missing commit's subject, as UTF-8 text.
    pub subject: String,
}
