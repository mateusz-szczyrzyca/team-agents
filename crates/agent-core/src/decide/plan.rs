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
use crate::ids::{AgentName, ItemId};
use crate::kinds::Role;
use crate::plan::{Plan, PlanError, Step, StepState};
use crate::state::{ItemPhase, PlanKeeping, PlanReason, PlanStage, RunEnd};
use crate::status::RunOutcome;

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
    /// The versions of the plan kept from earlier runs, oldest first; empty when none was.
    pub earlier: &'a [Plan],
    /// What the task keeps of its plan; `None` for a task created before plans.
    pub keeping: Option<&'a PlanKeeping>,
    /// The `[plan]` table of the configuration.
    pub policy: &'a Policy,
}

/// What the `DONE` of a run ends: the work the task's plan is about, or one turn of a coordinator whose members still
/// work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneEnds {
    /// The work the task was given, as far as its plan goes: the plan is to say so.
    Work,
    /// A turn of a coordinator while items wait for a member or are with one: its plan has steps for them still.
    Turn,
}

/// What the `DONE` of a task of `role` ends, in a team whose items are in `phases`.
///
/// A solo agent and a member end their item with `DONE`. A coordinator also ends a turn with `DONE` after it integrated
/// one member's branch while others work (`prompts/harness/team/bus-member-finished.md`): its `DONE` ends the work only
/// when no item is pending or in progress — every item is in review, in a fix pass, approved or done.
#[must_use]
pub fn done_ends<'a>(role: Role, phases: impl IntoIterator<Item = &'a ItemPhase>) -> DoneEnds {
    match role {
        Role::Coordinator => {
            let mut phases = phases.into_iter();
            if phases.any(|phase| match phase {
                ItemPhase::Pending | ItemPhase::InProgress { .. } => true,
                ItemPhase::InReview { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => false,
            }) {
                DoneEnds::Turn
            } else {
                DoneEnds::Work
            }
        }
        Role::Solo | Role::Member | Role::Worker | Role::Reviewer => DoneEnds::Work,
    }
}

/// What the `DONE` of a task of `role` ends, in a team whose items are in `phases`, when `works` says whether the member
/// an item in progress is with still works on it: an item whose member finished it and waits is the coordinator's to
/// integrate, not a member's work still to come. [`done_ends`] is this with every member still at work.
#[must_use]
pub fn done_ends_with<'a>(
    role: Role,
    phases: impl IntoIterator<Item = &'a ItemPhase>,
    works: impl Fn(&AgentName) -> bool,
) -> DoneEnds {
    let _ = works;
    done_ends(role, phases)
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
/// 6. a step of an earlier version that is gone, or renamed since the newest one, is a `LostStep`, before a step still
///    to do (`Unfinished`);
/// 7. a send-back below `[plan] bounces` plus what the human granted is a `SendBack`;
/// 8. otherwise the human is asked — `AskHuman` for a task that is not asked already, and for one that is, the same
///    answer: the task waits for the human's reply either way.
#[must_use]
pub fn decide_plan(facts: PlanFacts<'_>) -> PlanVerdict {
    if !matches!(
        facts.end,
        RunEnd::Reported {
            outcome: RunOutcome::Done
        }
    ) || !facts.policy.roles().contains(&facts.role)
    {
        return PlanVerdict::Accept;
    }
    let Some(keeping) = facts.keeping else {
        return PlanVerdict::Accept;
    };
    if keeping.stage() == PlanStage::Released {
        return PlanVerdict::Accept;
    }
    let limit = facts.policy.bounces().saturating_add(keeping.granted());
    if limit == 0 {
        return PlanVerdict::Accept;
    }
    let Some(reason) = fault(facts.read, facts.earlier) else {
        return PlanVerdict::Accept;
    };
    if keeping.sent_back() < limit {
        PlanVerdict::SendBack(reason)
    } else {
        PlanVerdict::AskHuman(reason)
    }
}

/// Why the plan a task read has a fault, the first of them being the one reported; `None` when it has none.
fn fault(read: &PlanRead, earlier: &[Plan]) -> Option<PlanReason> {
    match read {
        PlanRead::Missing => Some(PlanReason::Missing),
        PlanRead::Unreadable { line, detail } => Some(PlanReason::Unreadable {
            line: *line,
            detail: detail.clone(),
        }),
        PlanRead::Parsed(plan) => lost(earlier, plan).or_else(|| unfinished(plan)),
    }
}

/// The steps of `plan` that are still to do or in hand, in the order of the plan; `None` when none is.
fn unfinished(plan: &Plan) -> Option<PlanReason> {
    let steps: Vec<ItemId> = plan
        .steps()
        .filter(|step| matches!(step.state(), StepState::Todo | StepState::InHand))
        .map(Step::id)
        .collect();
    (!steps.is_empty()).then_some(PlanReason::Unfinished { steps })
}

/// The steps that `newer` no longer has though an earlier version had them — every earlier version counts, so that a
/// step lost by a version that was sent back stays lost — and the steps whose ID `newer` uses for another title than
/// the newest earlier version; each once, the lost ones first, in the order they first appeared. `None` when it loses
/// none.
fn lost(earlier: &[Plan], newer: &Plan) -> Option<PlanReason> {
    let mut steps: Vec<ItemId> = Vec::new();
    for version in earlier {
        for id in version.compare(newer).disappeared {
            if !steps.contains(&id) {
                steps.push(id);
            }
        }
    }
    if let Some(newest) = earlier.last() {
        for retitle in newest.compare(newer).retitled {
            if !steps.contains(&retitle.id) {
                steps.push(retitle.id);
            }
        }
    }
    (!steps.is_empty()).then_some(PlanReason::LostStep { steps })
}
