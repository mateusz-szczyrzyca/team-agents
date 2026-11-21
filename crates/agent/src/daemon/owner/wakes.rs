//! Waking the members the bus woke, for a message that waits for them or an item on their board.
//!
//! A member that is woken and does not read what woke it is woken again when its run ends, and so on. Three rules keep
//! that from going on for ever, and from costing a round its publication:
//!
//! - the lead whose fix response was taken is not woken until its branch is published: a lead that is queued is not
//!   waiting, and only a lead that waits has its branch published;
//! - a member is woken for the same message at most `limits.wake_ups_per_message` times; the message stays unread on the
//!   bus for the member's next run, and a newer message wakes it again;
//! - every wake-up is told in the log line of the run it starts, with the message and who sent it.

use std::num::NonZeroU64;

use agent_core::ids::{AgentName, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::state::{Findings, ItemPhase, Team, WakeDelivery};
use agent_runtime::agentbus::MessageNote;
use agent_runtime::process::error_chain;

use super::teams::Look;
use super::{Owner, OwnerError};
use crate::daemon::team::Why;

/// The wake-ups of one task for the newest message that waits for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WakeCount {
    /// The newest message that waited for the task at its last wake-up; `None` when the log named none (an item on the
    /// board woke it, or the log could not be read).
    message: Option<NonZeroU64>,
    /// How many runs the task was woken for it.
    count: u32,
    /// Whether the stop was told in the log already.
    stopped: bool,
    /// Whether the failure to read the log of the bus was told already; a read that works clears it.
    unreadable_told: bool,
}

/// What the log says of the run a wake-up starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WakeNote {
    /// The message that woke the task, when the log names one.
    pub(super) message: Option<MessageNote>,
    /// The number of this wake-up for that message.
    pub(super) wake_ups: u32,
}

/// What became of a wake-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wake {
    /// The task is queued to start.
    Queued,
    /// The task stays as it is: it is woken later, or not for this message again.
    Held,
}

impl Owner {
    /// Wakes `task`, a member of `team_id` that the bus woke, unless it leads a team whose fix response waits to be published or it was woken
    /// for this message as often as the configuration allows.
    ///
    /// # Errors
    /// [`OwnerError`] when the state is inconsistent.
    pub(super) async fn wake_member(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        look: &mut Look,
    ) -> Result<Wake, OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(Wake::Held);
        };
        let leads = team
            .crew()
            .roster()
            .first()
            .is_some_and(|(_, lead)| lead.task() == task);
        if leads && fix_response_taken(team) {
            return Ok(Wake::Held);
        }
        // The messages that wake a member are those to the name its task has on the bus, whoever holds it now.
        let agent: AgentName = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
            .member
            .bus_agent()
            .name()
            .parse()?;
        let known = self.wake_counts.remove(task).or_else(|| {
            self.parts.state.wake_delivery(task).map(|delivery| match delivery {
                WakeDelivery::Pending { message, count } => WakeCount {
                    message,
                    count,
                    stopped: false,
                    unreadable_told: false,
                },
                WakeDelivery::Delivered { message, count } => WakeCount {
                    message,
                    count,
                    stopped: true,
                    unreadable_told: false,
                },
            })
        });
        let (message, mut woken) = self.wake_message(&agent, team_id, task, known).await;
        let limit = self.parts.settings.config.limits().wake_ups_per_message;
        if woken.count >= limit {
            if !woken.stopped {
                woken.stopped = true;
                self.parts.state.record_wake_delivery(
                    task.clone(),
                    Some(WakeDelivery::Delivered {
                        message: woken.message,
                        count: woken.count,
                    }),
                );
                look.changed = true;
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    team = %team_id,
                    task = %task,
                    %agent,
                    message_id = ?message.as_ref().map(|note| note.id),
                    from = message.as_ref().map_or("", |note| note.from.as_str()),
                    wake_ups = woken.count,
                    "the member was woken for this message as often as limits.wake_ups_per_message allows and did not read it: it is not woken for it again"
                );
            }
            self.wake_counts.insert(task.clone(), woken);
            return Ok(Wake::Held);
        }
        woken.count = woken.count.saturating_add(1);
        self.wake_notes.insert(
            task.clone(),
            WakeNote {
                message,
                wake_ups: woken.count,
            },
        );
        self.parts.state.record_wake_delivery(
            task.clone(),
            Some(WakeDelivery::Pending {
                message: woken.message,
                count: woken.count,
            }),
        );
        look.changed = true;
        self.wake_counts.insert(task.clone(), woken);
        self.release(task, Why::Woken, look)?;
        Ok(Wake::Queued)
    }

    /// Reads the wake-up message, retaining its budget and reporting a bus failure once when lookup fails.
    async fn wake_message(
        &self,
        agent: &AgentName,
        team_id: &TeamId,
        task: &TaskId,
        known: Option<WakeCount>,
    ) -> (Option<MessageNote>, WakeCount) {
        match self.parts.bus.latest_message_for(agent, team_id).await {
            Ok(message) => {
                let id = message.as_ref().map(|note| note.id);
                let woken = match known {
                    Some(known) if known.message == id => WakeCount {
                        unreadable_told: false,
                        ..known
                    },
                    Some(_) | None => WakeCount {
                        message: id,
                        count: 0,
                        stopped: false,
                        unreadable_told: false,
                    },
                };
                (message, woken)
            }
            Err(error) => {
                // Without the log the message is not known: the count of the last known one goes on, so that a log that
                // cannot be read does not stretch the wake-ups, and the failure is told once.
                let mut woken = known.unwrap_or(WakeCount {
                    message: None,
                    count: 0,
                    stopped: false,
                    unreadable_told: false,
                });
                if !woken.unreadable_told {
                    woken.unreadable_told = true;
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        team = %team_id,
                        task = %task,
                        error = %error_chain(&error),
                        "the message that wakes a member cannot be read from the bus: the wake-up goes ahead without it"
                    );
                }
                (None, woken)
            }
        }
    }

    /// Forgets the wake-ups of `task`: it went to sleep with nothing waiting for it, so what woke it was read.
    pub(super) fn wake_settled(&mut self, task: &TaskId, look: &mut Look) {
        self.wake_counts.remove(task);
        if self.parts.state.wake_delivery(task).is_some() {
            self.parts.state.record_wake_delivery(task.clone(), None);
            look.changed = true;
        }
    }
}

/// Whether every pull request of `team` that is being fixed has its fix response taken, so that only the publication of
/// the branch is left to be done; false when none is being fixed.
fn fix_response_taken(team: &Team) -> bool {
    let mut fixing = team
        .items()
        .iter()
        .filter(|item| matches!(item.phase(), ItemPhase::Fixing { .. }))
        .peekable();
    fixing.peek().is_some()
        && fixing.all(|item| {
            item.findings()
                .and_then(Findings::disputes)
                .is_some_and(|disputes| disputes.answered_pass == crate::daemon::merge::pull_passes(team, item))
        })
}
