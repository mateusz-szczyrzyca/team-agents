//! The returns of a lead: what the daemon does when the lead of a team ends a run with `RETURNED: <item>`, the line that
//! says it sent the work of a member back instead of taking it (`docs/architecture.md`, "Rejections").
//!
//! The line is a claim; the daemon counts it only where its own facts agree (`decide::rejection::classify`): the item, or
//! its part, is held by a member whose last run for it ended `DONE`, and the same hand-over was not sent back before. The
//! return is recorded in the state before the lead's run is saved as ended, so that the two stand or fall together, with
//! what the lead told the member after the notice of the hand-over as its reason; it counts when the member is started
//! again.

use std::num::NonZeroU32;

use agent_core::decide::rejection::{Rejection, ReturnLine, classify, read_return, returned_targets, run_number};
use agent_core::ids::{AgentName, ItemId, RunId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::split::PartKey;
use agent_core::state::{ItemPhase, Timestamp};
use agent_core::status::RunOutcome;
use agent_runtime::agentbus::Reader;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::{Located, Owner, OwnerError};

/// What the lead's rejection says when the bus holds no message of the lead for the member after the notice of the
/// hand-over: the line is the only thing the lead said, and the member reads that it was returned from this.
const NO_REASON_ON_THE_BUS: &str = "the lead returned the work and left no message for the member on the bus";

impl Owner {
    /// Reads the `RETURNED:` lines of the final `message` of the lead `lead`, whose run ended `outcome`, and records, in
    /// the state and without saving it, a rejection for each target the daemon's facts say was handed over and is sent
    /// back. The caller saves the state together with the end of the lead's run.
    ///
    /// # Errors
    /// [`OwnerError`] when the team or its clock is gone or the state refuses a rejection.
    pub(super) async fn read_returns(
        &mut self,
        lead: &TaskId,
        located: &Located,
        outcome: RunOutcome,
        message: &str,
    ) -> Result<(), OwnerError> {
        for target in returned_targets(message) {
            let item = target.item();
            let Some(lead_started_at) = self
                .parts
                .state
                .find_task(lead)
                .and_then(|found| found.member.runs().last().map(agent_core::state::Run::started_at))
            else {
                continue;
            };
            let found = self
                .parts
                .state
                .team(&located.team)
                .and_then(|team| read_return(team, target, lead_started_at, outcome));
            let Some(found) = found else {
                tracing::debug!(team = %located.team, %item, "the lead returned an item that is not in its team");
                continue;
            };
            match classify(&ReturnLine::returned(&item), found.facts()) {
                Some(Rejection::Counted) => {
                    let who = found
                        .member()
                        .map(|member| (member.task().clone(), member.bus_agent().agent_name()));
                    let reason = match &who {
                        Some((member, bus_name)) => {
                            self.reason_on_the_bus(lead, &located.team, (member, bus_name)).await
                        }
                        None => NO_REASON_ON_THE_BUS.to_owned(),
                    };
                    let (part, of_run, started_again) = (found.part(), found.of_run().cloned(), found.started_again());
                    self.record_return(&located.team, item, reason, (part, of_run, started_again))?;
                }
                Some(Rejection::NotHandedOver | Rejection::AlreadyCounted) | None => {
                    // A return the facts do not support is no rejection and is never guessed into one.
                    log_decision(
                        "rejection",
                        &format!("team={}, item={item}", located.team),
                        "not counted",
                    );
                }
            }
        }
        Ok(())
    }
}

impl Owner {
    /// Marks the run of `task` that begins now, when it is a lead's: the newest message of the bus, after which what the
    /// lead says is its answer in this run. A bus that cannot be read leaves the run without a mark, and its returns
    /// without a reason: nothing older is taken for one.
    pub(super) async fn mark_lead_run(&mut self, task: &TaskId, role: Role) {
        if role != Role::Coordinator {
            return;
        }
        self.lead_marks.remove(task);
        match self.parts.bus.newest_message_id().await {
            Ok(Some(mark)) => {
                self.lead_marks.insert(task.clone(), mark);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(event = LogEvent::BusDown.as_str(), task = %task,
                    error = %error_chain(&error), "the bus cannot be read at the start of a lead's run");
            }
        }
    }

    /// What the lead `lead` told the member `(task, bus name)` during the run that ends now: the reason of the rejection,
    /// whole, whether the member read it or not. A bus that cannot be read, or that holds no message of the lead for the
    /// member since the run began, gives [`NO_REASON_ON_THE_BUS`]: the return stands, and its reason is what is known.
    async fn reason_on_the_bus(
        &self,
        lead: &TaskId,
        team: &TeamId,
        (member, bus_name): (&TaskId, &AgentName),
    ) -> String {
        let Some(lead_name) = self
            .parts
            .state
            .find_task(lead)
            .map(|found| found.member.bus_agent().name().to_owned())
        else {
            return NO_REASON_ON_THE_BUS.to_owned();
        };
        let Some(mark) = self.lead_marks.get(lead).copied() else {
            return NO_REASON_ON_THE_BUS.to_owned();
        };
        let said = self
            .parts
            .bus
            .said_after(
                Reader {
                    agent: bus_name,
                    team,
                    task: member,
                },
                &lead_name,
                Some(mark),
            )
            .await;
        match said {
            Ok(messages) if !messages.is_empty() => messages
                .into_iter()
                .map(|message| message.text)
                .collect::<Vec<_>>()
                .join("\n\n"),
            Ok(_) => NO_REASON_ON_THE_BUS.to_owned(),
            Err(error) => {
                tracing::warn!(event = LogEvent::BusDown.as_str(), team = %team, task = %member,
                    error = %error_chain(&error), "the reason of a rejection cannot be read from the bus");
                NO_REASON_ON_THE_BUS.to_owned()
            }
        }
    }

    /// Records that the lead returned `item` of `team` for `reason`, now; when the member `started_again` already — the
    /// message that sent the work back started it while the lead's run went on — the rejection is counted at once.
    fn record_return(
        &mut self,
        team: &TeamId,
        item: ItemId,
        reason: String,
        (part, of_run, started_again): (Option<PartKey>, Option<RunId>, Option<NonZeroU32>),
    ) -> Result<(), OwnerError> {
        let at = Timestamp::now(&self.parts.clock)?;
        let mut progress = self
            .parts
            .state
            .team_mut(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
        let mut progress = progress.item_mut(item).ok_or_else(|| OwnerError::LostItem {
            team: team.clone(),
            item,
        })?;
        progress
            .record_rejection_of(reason, at, part, of_run)
            .map_err(|source| OwnerError::Rejection { item, source })?;
        if let Some(run) = started_again {
            progress
                .next_run_started(run, at)
                .map_err(|source| OwnerError::Rejection { item, source })?;
        }
        log_decision("rejection", &format!("team={team}, item={item}"), "recorded");
        Ok(())
    }

    /// Counts the returns recorded for the items the member `task` holds, now that a run of it starts at `at`: the member
    /// was started again for them, which is what makes a return a rejection.
    ///
    /// # Errors
    /// [`OwnerError`] when the task or its team is gone, or a rejection was recorded after the run started.
    pub(super) fn confirm_returns(&mut self, task: &TaskId, at: Timestamp) -> Result<(), OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        if found.role != Role::Member {
            return Ok(());
        }
        let team = found.team.id().clone();
        let name = found.member.agent().name().to_owned();
        let run = run_number(found.member.runs().len());
        let held: Vec<ItemId> = found
            .team
            .items()
            .iter()
            .filter(|progress| match progress.phase() {
                ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
                    assignee.as_str() == name
                }
                ItemPhase::Pending
                | ItemPhase::InReview { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => false,
            })
            .map(agent_core::state::ItemProgress::id)
            .collect();
        for item in held {
            let counted = self
                .parts
                .state
                .team_mut(&team)
                .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?
                .item_mut(item)
                .ok_or_else(|| OwnerError::LostItem {
                    team: team.clone(),
                    item,
                })?
                .next_run_started(run, at)
                .map_err(|source| OwnerError::Rejection { item, source })?;
            if counted > 0 {
                log_decision("rejection", &format!("team={team}, item={item}, run={run}"), &counted);
            }
        }
        Ok(())
    }
}
