//! What a task keeps of its plan: how often its runs went back for it, and what the human answered (`docs/plan.md`).
//!
//! A member keeps a [`PlanKeeping`] from the moment this version creates its task: a task written before this version
//! has none (`None`), and its runs are never sent back for its plan. What is kept is a stage, two counts and the reason
//! of a send-back that no run has delivered yet, so a restart of the daemon loses nothing of it.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::ids::ItemId;

/// Where the task is with its plan: going back until it is clean, asked about, or released by the human.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStage {
    /// A run that ends done against an unclean plan goes back, until the limit.
    #[default]
    Open,
    /// The limit was reached and the human was asked about the plan in the task's questions `question`.
    Asked {
        /// The questions of the task the human was asked, as the task numbers them.
        question: NonZeroU32,
    },
    /// The human said to go on as it is: no run is sent back again, and the plan is still recorded.
    Released,
}

/// Why a run that ended done was sent back for its task's plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanReason {
    /// The task wrote no plan.
    Missing,
    /// The plan does not parse: `line` is the line of the fault when the reader names one.
    Unreadable {
        /// The line of the fault, counted from 1.
        line: Option<NonZeroU32>,
        /// What is wrong, in a line, as the reader says it.
        detail: String,
    },
    /// Steps are still to do or in hand.
    Unfinished {
        /// The steps, in the order of the plan.
        steps: Vec<ItemId>,
    },
    /// Steps the version before had are gone.
    LostStep {
        /// The steps, in the order of the version before.
        steps: Vec<ItemId>,
    },
}

/// What a task keeps of its plan: the count of its send-backs, what the human granted, the stage, and a reason that
/// waits for the run it goes to.
///
/// Made only by this module (private fields), through the operations below, so the counts never go back and a released
/// plan keeps no reason.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PlanKeeping {
    /// How many times the task's runs went back for its plan; in the state, so it survives a restart.
    sent_back: u32,
    /// Send-backs the human granted beyond `[plan] bounces`.
    granted: u32,
    /// Where the task is with its plan.
    stage: PlanStage,
    /// The reason of a send-back the next run of the task delivers; `None` when none waits, and not written then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pending: Option<PlanReason>,
}

impl PlanKeeping {
    /// How many times the task's runs went back for its plan.
    #[must_use]
    pub const fn sent_back(&self) -> u32 {
        self.sent_back
    }

    /// Send-backs the human granted beyond `[plan] bounces`.
    #[must_use]
    pub const fn granted(&self) -> u32 {
        self.granted
    }

    /// Where the task is with its plan.
    #[must_use]
    pub const fn stage(&self) -> PlanStage {
        self.stage
    }

    /// The reason the next run of the task delivers, when one waits.
    #[must_use]
    pub const fn pending(&self) -> Option<&PlanReason> {
        self.pending.as_ref()
    }

    /// Counts a send-back of the task, which its next run delivers with `reason`.
    pub(super) fn send_back(&mut self, reason: PlanReason) {
        self.sent_back = self.sent_back.saturating_add(1);
        self.pending = Some(reason);
    }

    /// Records that the human was asked about the plan in questions `question`.
    pub(super) fn asked(&mut self, question: NonZeroU32) {
        self.stage = PlanStage::Asked { question };
    }

    /// Counts one send-back the human granted beyond `[plan] bounces`.
    pub(super) fn grant_another(&mut self) {
        self.granted = self.granted.saturating_add(1);
    }

    /// The human said to go on as it is: no run is sent back again, and no reason waits.
    pub(super) fn release(&mut self) {
        self.stage = PlanStage::Released;
        self.pending = None;
    }
}
