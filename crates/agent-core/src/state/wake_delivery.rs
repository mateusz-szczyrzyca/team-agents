//! Persisted delivery budgets for messages that wake a task.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

/// Delivery of the latest wake-triggering message, retained even when its task is historical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum WakeDelivery {
    /// The task may still be woken for this message.
    Pending {
        /// The bus message, or none when only board work or an unreadable log identified the wake-up.
        message: Option<NonZeroU64>,
        /// Wake-ups already scheduled for it, including interrupted runs.
        count: u32,
    },
    /// The wake budget is exhausted; later runs may still read the message on the bus.
    Delivered {
        /// The bus message, or none when its identity could not be read.
        message: Option<NonZeroU64>,
        /// Wake-ups scheduled before delivery was considered complete.
        count: u32,
    },
}
