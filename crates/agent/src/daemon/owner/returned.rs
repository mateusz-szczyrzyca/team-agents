//! The returns of a lead: what the daemon does when the lead of a team ends a run with `RETURNED: <item>`, the line that
//! says it sent the work of a member back instead of taking it (`docs/architecture.md`, "Rejections").
//!
//! The line is a claim; the daemon counts it only where its own facts agree (`decide::rejection::classify`): the item is
//! held by a member whose last run ended `DONE` and which has not been started since. The return is recorded with the
//! lead's reason from the bus when the lead's run ends, and counted when the member's next run starts.

use agent_core::decide::rejection::{
    Answered, Facts, Held, LeadEnd, Rejection, ReturnLine, classify, returned_items, run_number, run_started_since,
};
use std::num::NonZeroU32;

use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::{ItemPhase, Member, Team, Timestamp};
use agent_core::status::RunOutcome;
use agent_runtime::agentbus::Reader;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::{Located, Owner, OwnerError};

/// What the lead's rejection says when the bus holds no message of the lead for the member: the line is the only thing
/// the lead said, and the member reads that it was returned from this.
const NO_REASON_ON_THE_BUS: &str = "the lead returned the work and left no message for the member on the bus";

/// What the daemon saw of an item the lead returned.
struct Returned {
    /// The task of the member that holds the item and the name the member has on its team's bus; `None` when no member
    /// holds it.
    member: Option<(TaskId, AgentName)>,
    /// What the daemon saw of the item and the member.
    facts: Facts,
    /// The number of the member's last run when the member was started again since the lead's run began.
    started_again: Option<NonZeroU32>,
}

impl Owner {
    /// Reads the `RETURNED:` lines of the final `message` of the lead `lead`, whose run ended `outcome`, and records a
    /// rejection for each item the daemon's facts say was handed over and is sent back.
    ///
    /// # Errors
    /// [`OwnerError`] when the team or its clock is gone, the state refuses a rejection or cannot be saved.
    pub(super) async fn read_returns(
        &mut self,
        lead: &TaskId,
        located: &Located,
        outcome: RunOutcome,
        message: &str,
    ) -> Result<(), OwnerError> {
        let items = returned_items(message);
        if items.is_empty() {
            return Ok(());
        }
        let mut recorded = false;
        for item in items {
            let Some(returned) = self.returned(&located.team, lead, item, outcome) else {
                tracing::debug!(team = %located.team, %item, "the lead returned an item that is not in its team");
                continue;
            };
            match classify(&ReturnLine::returned(&item), returned.facts) {
                Some(Rejection::Counted) => {
                    let reason = match &returned.member {
                        Some((member, bus_name)) => self.reason_on_the_bus(lead, &located.team, member, bus_name).await,
                        None => NO_REASON_ON_THE_BUS.to_owned(),
                    };
                    self.record_return(&located.team, item, reason, returned.started_again)?;
                    recorded = true;
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
        if recorded {
            self.save().await?;
        }
        Ok(())
    }

    /// The member that holds `item` of `team` and the facts about it when the run of the lead `lead` that ended `outcome`
    /// returned it; `None` when the team has no such item or the lead has no run.
    fn returned(&self, team: &TeamId, lead: &TaskId, item: ItemId, outcome: RunOutcome) -> Option<Returned> {
        let lead_started_at = self.parts.state.find_task(lead)?.member.runs().last()?.started_at();
        let team = self.parts.state.team(team)?;
        let progress = team.item(item)?;
        let assignee = match progress.phase() {
            ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => Some(assignee.as_str()),
            ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => {
                None
            }
        };
        let member = assignee.and_then(|name| member_named(team, name));
        let answered = if progress
            .rejections()
            .is_some_and(|recorded| recorded.iter().any(|rejection| rejection.counted_in().is_none()))
        {
            Answered::Yes
        } else {
            Answered::No
        };
        let (held, runs) = member.map_or((Held::Nobody, &[][..]), |member| (Held::ByTheMember, member.runs()));
        let facts = Facts::from_runs_before(held, answered, runs, lead_started_at).with_lead(LeadEnd::Settled(outcome));
        let started_again = run_started_since(runs, lead_started_at);
        let member = member.map(|member| (member.task().clone(), member.bus_agent().agent_name()));
        Some(Returned {
            member,
            facts,
            started_again,
        })
    }
}

/// The member of the crew of `team` whose agent is called `name`.
fn member_named<'a>(team: &'a Team, name: &str) -> Option<&'a Member> {
    team.crew()
        .roster()
        .into_iter()
        .find(|(role, member)| *role == Role::Member && member.agent().name() == name)
        .map(|(_, member)| member)
}

impl Owner {
    /// What the lead `lead` last told the member `member`, whose name on the bus of `team` is `bus_name`: the reason of
    /// the rejection, which the member may have read already, since the message is what started it again. A bus that
    /// cannot be read, or that holds no message of the lead for the member, gives [`NO_REASON_ON_THE_BUS`]: the return
    /// stands, and its reason is what is known.
    async fn reason_on_the_bus(&self, lead: &TaskId, team: &TeamId, member: &TaskId, bus_name: &AgentName) -> String {
        let Some(lead_name) = self
            .parts
            .state
            .find_task(lead)
            .map(|found| found.member.bus_agent().name().to_owned())
        else {
            return NO_REASON_ON_THE_BUS.to_owned();
        };
        let said = self
            .parts
            .bus
            .last_message_from(
                Reader {
                    agent: bus_name,
                    team,
                    task: member,
                },
                &lead_name,
            )
            .await;
        match said {
            Ok(Some(message)) => message.text,
            Ok(None) => NO_REASON_ON_THE_BUS.to_owned(),
            Err(error) => {
                tracing::warn!(event = LogEvent::HistoryRecordIncomplete.as_str(), team = %team, task = %member,
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
        started_again: Option<NonZeroU32>,
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
            .record_rejection(reason, at)
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
