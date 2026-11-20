//! Durable proposal input and correction progress, ready for later daemon integration.

use super::proposal::ReadProposalError;
use crate::decide::split::approval::ProposalFault;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Progress of a proposal that has not become an approved split.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalStage {
    /// A saved proposal awaits a decision.
    Pending,
    /// The coordinator must correct this recorded fault.
    Correction {
        /// Rejection delivered on the coordinator's next run.
        fault: ProposalFault,
    },
    /// A human question is already written; do not ask again after a restart.
    Asked {
        /// Existing questions file's nonzero number.
        question: NonZeroU32,
    },
}

/// Bounded raw input is kept even when it cannot yet be parsed as a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "KeepingWire", into = "KeepingWire")]
pub struct ProposalKeeping {
    /// Input needed to resume a decision after a restart.
    text: String,
    /// Completed send-backs; replacement input never resets this count.
    corrections: u32,
    /// Decision progress and any undelivered correction or existing question.
    stage: ProposalStage,
}

/// Persisted representation; input size is checked on every read.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeepingWire {
    /// Raw proposal, including malformed input awaiting correction.
    text: String,
    /// Completed send-backs.
    corrections: u32,
    /// Decision progress.
    stage: ProposalStage,
}
impl TryFrom<KeepingWire> for ProposalKeeping {
    type Error = ReadProposalError;
    fn try_from(wire: KeepingWire) -> Result<Self, Self::Error> {
        let mut keeping = Self::new(wire.text)?;
        keeping.corrections = wire.corrections;
        keeping.stage = wire.stage;
        Ok(keeping)
    }
}
impl From<ProposalKeeping> for KeepingWire {
    fn from(keeping: ProposalKeeping) -> Self {
        Self {
            text: keeping.text,
            corrections: keeping.corrections,
            stage: keeping.stage,
        }
    }
}
impl ProposalKeeping {
    /// Stores bounded raw input before a decision is made.
    ///
    /// # Errors
    /// Returns [`ReadProposalError::TooLarge`] when the input exceeds the questions-file bound.
    pub fn new(text: String) -> Result<Self, ReadProposalError> {
        if text.len() > crate::questions::MAX_QUESTIONS_BYTES {
            return Err(ReadProposalError::TooLarge);
        }
        Ok(Self {
            text,
            corrections: 0,
            stage: ProposalStage::Pending,
        })
    }
    /// Saved input, including syntax faults.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Completed corrections, retained across submissions.
    pub const fn corrections(&self) -> u32 {
        self.corrections
    }
    /// Current decision progress.
    pub const fn stage(&self) -> &ProposalStage {
        &self.stage
    }
    /// Replaces input atomically while preserving the correction budget.
    ///
    /// # Errors
    /// Returns [`ReadProposalError::TooLarge`] without mutation if the input is too large.
    pub fn resubmit(&mut self, text: String) -> Result<(), ReadProposalError> {
        let replacement = Self::new(text)?;
        self.text = replacement.text;
        self.stage = replacement.stage;
        Ok(())
    }
    /// Records one rejection and its undelivered reason; the count saturates rather than wrapping.
    pub fn send_back(&mut self, fault: ProposalFault) {
        self.corrections = self.corrections.saturating_add(1);
        self.stage = ProposalStage::Correction { fault };
    }
    /// Records the question already written for the human.
    pub fn asked(&mut self, question: NonZeroU32) {
        self.stage = ProposalStage::Asked { question };
    }
}
#[cfg(test)]
mod tests;
