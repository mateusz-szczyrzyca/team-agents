//! The human in the round: the fix pass the lead is given, the question put to the human when the fix passes are used up,
//! and the fix pass their answer gives.

use agent_core::decide::round::RoundPolicy;
use agent_core::ids::TeamId;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::EventKind;
use agent_core::state::{ItemPhase, Timestamp};
use agent_runtime::process::error_chain;

use super::super::teams::Round;
use crate::daemon::merge::{
    Asking, Decision, Exhausted, Next, after_answer, asking, decision, exhausted, fix_delivery,
};
use crate::daemon::team::Why;

use super::super::{Owner, OwnerError};
use super::{Announcement, Lead, Pulls};

impl Owner {
    /// Gives the lead the fix pass that waits for it: its task is queued, and its prompt has the findings.
    pub(super) fn deliver_fix_passes(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        if let Some(lead) = fix_delivery(team) {
            self.release(&lead, Why::FixPass, round)?;
        }
        Ok(())
    }

    /// What the pull requests that wait for the human need: the lead is woken to put the question; a lead that ran and put
    /// none leaves the human told directly; an answer gives one more fix pass.
    pub(super) fn ask_the_human(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let waiting = exhausted(team);
        if waiting.is_empty() {
            return Ok(());
        }
        let asks = asking(team);
        let answered: Vec<Exhausted> = {
            let roster = team.crew().roster();
            roster.first().map_or_else(Vec::new, |(_, lead)| {
                waiting
                    .iter()
                    .filter(|waits| decision(lead, waits.requested_at) == Decision::Answered)
                    .cloned()
                    .collect()
            })
        };
        let lead = self.lead_of(team_id)?;
        for ask in asks {
            match ask {
                Asking::Wake { lead, .. } => self.release(&lead, Why::AskHuman, round)?,
                Asking::Notify { exhausted } => self.tell_decision(pulls, team_id, &lead, &exhausted)?,
            }
        }
        for waits in &answered {
            self.start_pass_after_answer(team_id, &lead, waits, round)?;
        }
        Ok(())
    }

    /// Tells the human, once, that the reviewers still ask for changes and nobody put the question to them.
    pub(super) fn tell_decision(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        waits: &Exhausted,
    ) -> Result<(), OwnerError> {
        let announcement = Announcement::Decision {
            team: team_id.clone(),
            pr: waits.pr.number,
            requested_at: waits.requested_at,
        };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let values = Variables::new()
            .with("team", team_id.as_str())
            .and_then(|values| values.with("pr", waits.pr.number.to_string()))
            .and_then(|values| values.with("passes", waits.passes.to_string()));
        self.tell_human(pulls, AutopilotPrompt::NoticeDecision, values, lead, now);
        Ok(())
    }

    /// The human answered the question about `waits`: the round machine gives the lead one more fix pass.
    pub(super) fn start_pass_after_answer(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        waits: &Exhausted,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        let mut moving = Vec::new();
        for id in &waits.items {
            let Some(progress) = team.item(*id) else {
                continue;
            };
            let next = after_answer(progress, policy);
            match next {
                Ok(Next::FixPass { .. }) => moving.push(*id),
                Ok(other) => {
                    tracing::warn!(team = %team_id, item = %id, ?other, "the round does not take the answer of the human as a fix pass");
                }
                Err(error) => {
                    tracing::warn!(team = %team_id, item = %id, error = %error_chain(&error), "the round does not take the answer of the human");
                }
            }
        }
        for id in moving {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: id,
            })?;
            let after = ItemPhase::Fixing {
                assignee: lead.agent.clone(),
                branch: waits.pr.branch.clone(),
                pr: waits.pr.clone(),
            };
            let status = after.kind().status();
            item.advance(after).map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: id,
                source,
            })?;
            round.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: id,
                status,
            });
            round.changed = true;
        }
        self.release(&lead.task, Why::FixPass, round)
    }
}
