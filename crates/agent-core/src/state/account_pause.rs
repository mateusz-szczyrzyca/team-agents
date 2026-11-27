//! The pause of an account: until when none of its agents starts a run, and for which limit.

use serde::{Deserialize, Serialize};

use super::Timestamp;
use crate::decide::limit::Limit;

/// An account whose provider refused a run: no run of any of its agents starts before `until`.
///
/// Written by the daemon when the first refused run of the account is settled and forgotten when `until` has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPause {
    /// When the pause ends: the run that is refused after it is the first that may start.
    pub until: Timestamp,
    /// What the refusal that started the pause showed of its limit.
    pub reason: Limit,
}
