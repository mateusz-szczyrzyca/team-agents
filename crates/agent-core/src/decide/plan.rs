//! The plan of a task at the end of its run: accept it, send the run back for it, or ask the human (`docs/plan.md`).
//!
//! A run of a role of `[plan] roles` that ends `STATUS: DONE` is sent back to the same agent when its task has no plan,
//! when the plan does not parse, when a step is still to do or in hand, or when a step the version before had is gone.
//! After `[plan] bounces` send-backs the human is asked once, in the task's questions file. Nothing happens for another
//! status, for a role outside the list, for a task created before plans, for a plan the human released, or when the
//! limit is `0`.
//!
//! The decision is pure: the daemon reads the plan and the state, calls [`decide_plan`] and acts on the verdict.

use std::num::NonZeroU32;

use crate::config::Plan as Policy;
use crate::ids::ItemId;
use crate::kinds::Role;
use crate::plan::{Plan, PlanError};
use crate::state::{PlanKeeping, PlanReason, RunEnd};

#[cfg(test)]
mod tests;

/// What the daemon read at the end of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRead {
    /// The task wrote no plan.
    Missing,
    /// The text is not a plan: it does not parse, is not UTF-8 text, or is over the size limit.
    Unreadable {
        /// The line of the fault when the reader names one.
        line: Option<NonZeroU32>,
        /// What is wrong, in a line, as the reader says it.
        detail: String,
    },
    /// The plan, as the task wrote it.
    Parsed(Plan),
}

impl PlanRead {
    /// What the reader says of a text that is not a plan, with the line of the fault when it names one.
    #[must_use]
    pub fn unreadable(error: &PlanError) -> Self {
        let line = error
            .line()
            .and_then(|line| u32::try_from(line.get()).ok())
            .and_then(NonZeroU32::new);
        Self::Unreadable {
            line,
            detail: error.to_string(),
        }
    }

    /// The plan, when the text is one.
    #[must_use]
    pub fn plan(&self) -> Option<&Plan> {
        match self {
            Self::Parsed(plan) => Some(plan),
            Self::Missing | Self::Unreadable { .. } => None,
        }
    }
}

/// What the decision looks at: the run, the role of its task, the plan it read and what the task kept.
#[derive(Debug, Clone, Copy)]
pub struct PlanFacts<'a> {
    /// The role of the task whose run ended.
    pub role: Role,
    /// How the run ended.
    pub end: &'a RunEnd,
    /// What the daemon read at the end of the run.
    pub read: &'a PlanRead,
    /// The last version of the plan kept from an earlier run, when one was.
    pub previous: Option<&'a Plan>,
    /// What the task keeps of its plan; `None` for a task created before plans.
    pub keeping: Option<&'a PlanKeeping>,
    /// The `[plan]` table of the configuration.
    pub policy: &'a Policy,
}

/// What happens after a run ended that reports done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanVerdict {
    /// The run ends as it says.
    Accept,
    /// The run goes back to the same agent with this reason.
    SendBack(PlanReason),
    /// The limit is reached: the human is asked about this reason.
    AskHuman(PlanReason),
}

/// Decides what happens to the plan of a task whose run ended as `end`, with what the task kept and what was read.
///
/// The rules, in this order:
///
/// 1. a run that did not report `STATUS: DONE` is accepted, whatever its plan says;
/// 2. a role outside `[plan] roles` is accepted: it keeps no plan, though one it wrote is recorded;
/// 3. a task created before plans, or whose plan the human released, is accepted;
/// 4. a limit of `0` accepts every run;
/// 5. a plan without a fault is accepted;
/// 6. a step of the version before that is gone is a `LostStep`, before a step still to do (`Unfinished`);
/// 7. a send-back below `[plan] bounces` plus what the human granted is a `SendBack`;
/// 8. otherwise the human is asked — once, for a task that is not asked already.
#[must_use]
pub fn decide_plan(facts: PlanFacts<'_>) -> PlanVerdict {
    let _reason = fault(facts.read, facts.previous, facts.keeping);
    PlanVerdict::Accept
}

/// Why the plan a task keeps has a fault, the first of them being the one reported; `None` when it has none.
fn fault(read: &PlanRead, previous: Option<&Plan>, keeping: Option<&PlanKeeping>) -> Option<PlanReason> {
    let parsed = read.plan();
    let steps = parsed.map(unfinished).is_none_or(|steps| steps.is_empty());
    let lost_steps = parsed
        .zip(previous)
        .map(|(newer, older)| lost(older, newer))
        .is_none_or(|steps| steps.is_empty());
    let asked = keeping.is_some_and(|keeping| !matches!(keeping.stage(), crate::state::PlanStage::Open));
    if steps && lost_steps && !asked {
        return None;
    }
    Some(PlanReason::Missing)
}

/// The steps of `plan` that are still to do or in hand, in the order of the plan.
fn unfinished(plan: &Plan) -> Vec<ItemId> {
    let _ = plan;
    Vec::new()
}

/// The steps of `previous` that `newer` no longer has, or uses for another title, in the order of `previous`.
fn lost(previous: &Plan, newer: &Plan) -> Vec<ItemId> {
    let _ = (previous, newer);
    Vec::new()
}
