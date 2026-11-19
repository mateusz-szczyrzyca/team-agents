//! Waking the members the bus woke, for a message that waits for them or an item on their board.
//!
//! A member that is woken and does not read what woke it is woken again when its run ends, and so on. Three rules keep
//! that from going on for ever, and from costing a round its publication:
//!
//! - the lead that waits while a pull request is being fixed is not woken yet: the fix pass, its response or the
//!   publication of its branch starts a run of it soon enough, and a lead that is queued is not waiting, while only a
//!   lead that waits has its branch published;
//! - a member is woken for the same message at most `limits.wake_ups_per_message` times; the message stays unread on the
//!   bus for the member's next run, and a newer message wakes it again;
//! - every wake-up is told in the log line of the run it starts, with the message and who sent it.

use std::num::NonZeroU64;

use agent_core::ids::{AgentName, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::state::ItemPhase;
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
    /// Wakes `task`, a member of `team_id` that the bus woke, unless it leads a team whose pull request is being fixed or it was woken
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
        let fixing = team
            .items()
            .iter()
            .any(|item| matches!(item.phase(), ItemPhase::Fixing { .. }));
        if leads && fixing {
            return Ok(Wake::Held);
        }
        let agent: AgentName = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
            .member
            .agent()
            .name()
            .parse()?;
        let message = match self.parts.bus.latest_message_for(&agent, team_id).await {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    team = %team_id,
                    task = %task,
                    error = %error_chain(&error),
                    "the message that wakes a member cannot be read from the bus: the wake-up goes ahead without it"
                );
                None
            }
        };
        let id = message.as_ref().map(|note| note.id);
        let limit = self.parts.settings.config.limits().wake_ups_per_message;
        let mut woken = match self.wake_counts.remove(task) {
            Some(known) if known.message == id => known,
            Some(_) | None => WakeCount {
                message: id,
                count: 0,
                stopped: false,
            },
        };
        if woken.count >= limit {
            if !woken.stopped {
                woken.stopped = true;
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
        self.wake_counts.insert(task.clone(), woken);
        self.release(task, Why::Woken, look)?;
        Ok(Wake::Queued)
    }

    /// Forgets the wake-ups of `task`: it went to sleep with nothing waiting for it, so what woke it was read.
    pub(super) fn wake_settled(&mut self, task: &TaskId) {
        self.wake_counts.remove(task);
    }
}
