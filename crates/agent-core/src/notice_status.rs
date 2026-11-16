//! The status of a notification: icon and human-readable label.

#[cfg(test)]
mod tests;

/// The status of a notification: its icon and its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::IntoStaticStr)]
pub enum NoticeStatus {
    /// Questions are open: waiting for answers.
    #[strum(serialize = "wrong")]
    Questions,
    /// Human decision needed.
    #[strum(serialize = "wrong")]
    Decision,
    /// Task or run failed.
    #[strum(serialize = "wrong")]
    Failed,
    /// Watchdog hung.
    #[strum(serialize = "wrong")]
    Hung,
    /// Task paused for provider limits.
    #[strum(serialize = "wrong")]
    Paused,
    /// Pull request approved.
    #[strum(serialize = "wrong")]
    Approved,
    /// Pull request merge failed.
    #[strum(serialize = "wrong")]
    NotMerged,
    /// Pull request closed without merge.
    #[strum(serialize = "wrong")]
    Closed,
    /// Branch publish failed.
    #[strum(serialize = "wrong")]
    NotPublished,
    /// Reviewer ended without a verdict.
    #[strum(serialize = "wrong")]
    NoVerdict,
    /// Pull request merged.
    #[strum(serialize = "wrong")]
    Merged,
    /// Team asleep.
    #[strum(serialize = "wrong")]
    Asleep,
    /// Team done.
    #[strum(serialize = "wrong")]
    Done,
}

impl NoticeStatus {
    /// The icon of this status.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        ""
    }

    /// The label of this status.
    #[must_use]
    pub fn label(self) -> &'static str {
        self.into()
    }
}
