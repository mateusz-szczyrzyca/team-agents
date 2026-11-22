//! The returns of a lead: what the daemon does when the lead of a team ends a run with `RETURNED: <item>`, the line that
//! says it sent the work of a member back instead of taking it (`docs/architecture.md`, "Rejections").
//!
//! The line is a claim; the daemon counts it only where its own facts agree (`decide::rejection::classify`): the item, or
//! its part, is held by a member whose last run for it ended `DONE`, and the same hand-over was not sent back before. The
//! return is recorded in the state before the lead's run is saved as ended, so that the two stand or fall together, with
//! what the lead told the member after the notice of the hand-over as its reason; it counts when the member is started
//! again.

use std::num::NonZeroU32;

use agent_core::decide::rejection::{
    Answered, Facts, Held, LeadEnd, Rejection, ReturnLine, ReturnTarget, classify, handed_over_run, returned_targets,
    run_number, run_started_since, runs_of_round,
};
use agent_core::ids::{AgentName, ItemId, RunId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::split::PartKey;
use agent_core::state::{ItemPhase, Member, RoundLife, Team, Timestamp};
use agent_core::status::RunOutcome;
use agent_runtime::agentbus::{Finished, Reader};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::{Located, Owner, OwnerError};

/// What the lead's rejection says when the bus holds no message of the lead for the member after the notice of the
/// hand-over: the line is the only thing the lead said, and the member reads that it was returned from this.
const NO_REASON_ON_THE_BUS: &str = "the lead returned the work and left no message for the member on the bus";

/// What the daemon saw of an item the lead returned.
struct Returned {
    /// The member that holds the item: its task, its name on its team's bus and its agent name; `None` when no member
    /// holds it.
    member: Option<(TaskId, AgentName, String)>,
    /// What the daemon saw of the item and the member.
    facts: Facts,
    /// The part of the item the work belongs to: the part of the round that is open for the item.
    part: Option<PartKey>,
    /// The run of the member that ended `DONE` and is sent back.
    of_run: Option<RunId>,
    /// The number of the member's last run when the member was started again since the lead's run began.
    started_again: Option<NonZeroU32>,
}

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
            let Some(returned) = self.returned(&located.team, lead, target, outcome) else {
                tracing::debug!(team = %located.team, %item, "the lead returned an item that is not in its team");
                continue;
            };
            match classify(&ReturnLine::returned(&item), returned.facts) {
                Some(Rejection::Counted) => {
                    let reason = match &returned.member {
                        Some((member, bus_name, agent)) => {
                            self.reason_on_the_bus(lead, &located.team, item, (member, bus_name, agent))
                                .await
                        }
                        None => NO_REASON_ON_THE_BUS.to_owned(),
                    };
                    self.record_return(&located.team, item, reason, &returned)?;
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

    /// The member that holds the item `target` names in `team` and the facts about it when the run of the lead `lead`
    /// that ended `outcome` returned it; `None` when the team has no such item or the lead has no run.
    fn returned(&self, team: &TeamId, lead: &TaskId, target: ReturnTarget, outcome: RunOutcome) -> Option<Returned> {
        let lead_started_at = self.parts.state.find_task(lead)?.member.runs().last()?.started_at();
        let team = self.parts.state.team(team)?;
        let progress = team.item(target.item())?;
        let round = team
            .rounds()
            .iter()
            .find(|round| round.item() == Some(progress.id()) && matches!(round.life(), RoundLife::Open));
        let part = round.and_then(agent_core::state::Round::part);
        let assignee = match progress.phase() {
            ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => Some(assignee.as_str()),
            ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => {
                None
            }
        };
        // A line that names a part counts only for the part the item is at: the work of another part is not what is open.
        let names_the_open_part = target.part().is_none() || target.part() == part;
        let member = assignee
            .and_then(|name| member_named(team, name))
            .filter(|_| names_the_open_part && round.is_some());
        // The runs of the item's own round: the member's runs for items it held before are no hand-over of this one.
        let runs = match (member, round) {
            (Some(member), Some(round)) => runs_of_round(member.runs(), round.id()),
            _ => Vec::new(),
        };
        let of_run = handed_over_run(&runs).cloned();
        let sent_back_already = of_run.as_ref().is_some_and(|run| progress.was_rejected(run))
            || progress
                .rejections()
                .is_some_and(|recorded| recorded.iter().any(|rejection| rejection.counted_in().is_none()));
        let answered = if sent_back_already { Answered::Yes } else { Answered::No };
        let held = if member.is_some() {
            Held::ByTheMember
        } else {
            Held::Nobody
        };
        let facts =
            Facts::from_runs_before(held, answered, &runs, lead_started_at).with_lead(LeadEnd::Settled(outcome));
        // The number is the position among all the runs of the member's task, as `confirm_returns` counts it.
        let started_again = member
            .and_then(|member| run_started_since(&runs, lead_started_at).map(|_| run_number(member.runs().len())));
        let member = member.map(|member| {
            (
                member.task().clone(),
                member.bus_agent().agent_name(),
                member.agent().name().to_owned(),
            )
        });
        Some(Returned {
            member,
            facts,
            part,
            of_run,
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
    /// What the lead `lead` told the member `(task, bus name, agent name)` directly after the notice that it finished
    /// `item` in `team`: the reason of the rejection, whole, whether the member read it or not. A bus that cannot be read,
    /// or that holds no such notice or no message after it, gives [`NO_REASON_ON_THE_BUS`]: the return stands, and its
    /// reason is what is known.
    async fn reason_on_the_bus(
        &self,
        lead: &TaskId,
        team: &TeamId,
        item: ItemId,
        (member, bus_name, agent): (&TaskId, &AgentName, &str),
    ) -> String {
        let Some(lead_name) = self
            .parts
            .state
            .find_task(lead)
            .map(|found| found.member.bus_agent().name().to_owned())
        else {
            return NO_REASON_ON_THE_BUS.to_owned();
        };
        let item = item.to_string();
        let said = self
            .parts
            .bus
            .said_since_finished(
                Reader {
                    agent: bus_name,
                    team,
                    task: member,
                },
                &lead_name,
                Finished {
                    lead: &lead_name,
                    member: agent,
                    item: &item,
                },
            )
            .await;
        match said {
            Ok(Some(messages)) if !messages.is_empty() => messages
                .into_iter()
                .map(|message| message.text)
                .collect::<Vec<_>>()
                .join("\n\n"),
            Ok(Some(_) | None) => NO_REASON_ON_THE_BUS.to_owned(),
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
        returned: &Returned,
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
            .record_rejection_of(reason, at, returned.part, returned.of_run.clone())
            .map_err(|source| OwnerError::Rejection { item, source })?;
        if let Some(run) = returned.started_again {
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
