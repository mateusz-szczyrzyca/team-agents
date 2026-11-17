//! The status of a notification: icon and human-readable label.

#[cfg(test)]
mod tests;

/// The status of a notification: its icon and its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::IntoStaticStr)]
pub enum NoticeStatus {
    /// Questions are open: waiting for answers.
    #[strum(serialize = "waiting for answers")]
    Questions,
    /// Human decision needed.
    #[strum(serialize = "needs your decision")]
    Decision,
    /// Task or run failed.
    #[strum(serialize = "failed")]
    Failed,
    /// Watchdog hung.
    #[strum(serialize = "hung")]
    Hung,
    /// Task paused for provider limits.
    #[strum(serialize = "paused")]
    Paused,
    /// Pull request approved.
    #[strum(serialize = "approved: ready to merge")]
    Approved,
    /// Pull request merge failed.
    #[strum(serialize = "not merged")]
    NotMerged,
    /// Pull request closed without merge.
    #[strum(serialize = "pull request closed")]
    Closed,
    /// Branch publish failed.
    #[strum(serialize = "not published")]
    NotPublished,
    /// Reviewer ended without a verdict.
    #[strum(serialize = "review without a verdict")]
    NoVerdict,
    /// Pull request merged.
    #[strum(serialize = "merged")]
    Merged,
    /// Team asleep.
    #[strum(serialize = "team asleep")]
    Asleep,
    /// Team done.
    #[strum(serialize = "done")]
    Done,
}

impl NoticeStatus {
    /// The icon of this status.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Questions | Self::Decision => "❓",
            Self::Failed | Self::NotMerged => "❌",
            Self::Hung => "⌛",
            Self::Paused => "⏸",
            Self::Approved => "✅✅",
            Self::Closed => "🚫",
            Self::NotPublished => "📕",
            Self::NoVerdict => "❗",
            Self::Merged => "🟣",
            Self::Asleep => "💤",
            Self::Done => "✅",
        }
    }

    /// The label of this status.
    #[must_use]
    pub fn label(self) -> &'static str {
        self.into()
    }
}
