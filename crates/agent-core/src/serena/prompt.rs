//! The reminder of Serena: the values of its text, for the harness to render from its prompts.

use super::SessionTally;
use crate::prompt::{HarnessPrompt, Variables, VariablesError};

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;

/// The text of the reminder, and the values it renders with: the count of searches of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderText {
    /// The text of the catalog the harness renders.
    text: HarnessPrompt,
    /// Its values.
    values: Variables,
}

impl ReminderText {
    /// The reminder of a session that made `tally`'s searches of code without Serena.
    ///
    /// # Errors
    /// [`ReminderTextError::Variables`] when the count is no value of the text.
    pub fn of(tally: &SessionTally) -> Result<Self, ReminderTextError> {
        let values = Variables::new()
            .with("count", tally.searches().to_string())
            .map_err(|source| ReminderTextError::Variables { source })?;
        Ok(Self {
            text: HarnessPrompt::SerenaRemind,
            values,
        })
    }

    /// The text whose values these are.
    #[must_use]
    pub const fn text(&self) -> HarnessPrompt {
        self.text
    }

    /// The values of the text.
    #[must_use]
    pub const fn values(&self) -> &Variables {
        &self.values
    }
}

/// Why the values of the text of the reminder cannot be made. Rendering the text is the harness's own business and has
/// its own error where the text is rendered.
#[derive(Debug, thiserror::Error)]
pub enum ReminderTextError {
    /// The count is not a value the text of the reminder takes.
    #[error("the count of searches is no value of the text of the reminder")]
    Variables {
        /// What the name refused.
        source: VariablesError,
    },
}
