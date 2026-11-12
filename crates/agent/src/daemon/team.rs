//! The life of a team, as decisions: how a `run` forms a team, which item goes to which member, what each member's
//! state on the bus should be, when a team is finished, and what a member's run ending tells the coordinator.
//!
//! Everything here is a function of the state (and the configuration): none of it touches git, the bus or a file. The
//! owner of the state carries out what these functions decide (`owner/teams.rs`), so each decision is tested without a
//! process.

mod assign;
mod close;
mod form;
mod jobs;
mod prompts;
mod react;
#[cfg(test)]
mod support;
mod sync;

pub(super) use self::assign::{Assigned, AssignmentError, assignable, item_branch, start_item, task_branch};
pub(super) use self::close::{closing_transitions, is_finished, is_held};
pub(super) use self::form::{FormError, Formed, Workspace, form};
pub(super) use self::react::{BusNotice, RequestOutcome, notice_for, request_outcome};
pub(super) use self::sync::{Reconciliation, desired_bus_state, reconcile};
