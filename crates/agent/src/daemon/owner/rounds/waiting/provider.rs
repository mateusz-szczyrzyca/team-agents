//! The wait of a round whose task is paused by its provider: which agent, and until when.

use agent_core::ids::AgentName;
use agent_core::kinds::Role;
use agent_core::state::{Member, Round, RunState, Team, Timestamp};
use agent_core::status::TaskStatus;

use crate::daemon::owner::Owner;
use crate::daemon::settle::limit_of;

/// How far from now a moment of a wait line is shown with its date: nearer than this, the time of the day tells it.
const SAME_DAY: time::Duration = time::Duration::hours(24);

/// A task of a round that its provider refused: its agent and the moment the agent returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::owner) struct ProviderPause {
    /// The agent whose provider refused the run.
    pub(in crate::daemon::owner) agent: AgentName,
    /// When the agent returns to the pool.
    pub(in crate::daemon::owner) until: Timestamp,
}

impl Owner {
    /// The pause of the provider that the idle `round` waits for, or `None` when it waits for anybody else.
    ///
    /// A task is paused by its provider when its last run ended refused, the task is paused and its agent is still refused at
    /// `now`: a pause the human asked for ends its run another way, and a refusal that is over holds nothing. The round
    /// waits at the provider only when every task that holds it is paused so; a question that is open or a task that is
    /// stopped or failed is the human's. A reviewer whose review is no longer asked holds nothing.
    pub(in crate::daemon::owner) fn provider_pause(
        &self,
        team: &Team,
        round: &Round,
        now: Timestamp,
    ) -> Option<ProviderPause> {
        let mut earliest: Option<ProviderPause> = None;
        for (role, member) in team.members_in(round.id()) {
            let stuck = matches!(
                member.status(),
                TaskStatus::Questions | TaskStatus::Paused | TaskStatus::Stopped | TaskStatus::Failed
            );
            let holds = member.open_question().is_some()
                || (stuck && (role != Role::Reviewer || crate::daemon::merge::review_asked(team, member.task())));
            if !holds {
                continue;
            }
            let pause = self.refused_pause(member, now)?;
            if earliest.as_ref().is_none_or(|known| pause.until < known.until) {
                earliest = Some(pause);
            }
        }
        earliest
    }

    /// The pause of `member` by its provider at `now`, or `None` when it is paused for another reason.
    fn refused_pause(&self, member: &Member, now: Timestamp) -> Option<ProviderPause> {
        if member.status() != TaskStatus::Paused || member.open_question().is_some() {
            return None;
        }
        let Some(RunState::Ended { at, end }) = member.runs().last().map(agent_core::state::Run::state) else {
            return None;
        };
        limit_of(end, *at)?;
        let agent = member.agent().agent_name();
        let until = self.parts.state.refused_until(&agent).filter(|until| now < *until)?;
        Some(ProviderPause { agent, until })
    }

    /// The wait line of a round that waits at the provider: the agent and the time it returns.
    pub(in crate::daemon::owner) fn provider_line(&self, pause: &ProviderPause, now: Timestamp) -> String {
        agent_core::decide::wait::provider_line(&pause.agent, &self.local_moment(pause.until, Some(now)))
    }

    /// `at` as the host's zone shows it in a wait line, with no offset: the time of the day when it is less than a day from
    /// `now`, and with its date otherwise, or when `now` is not known.
    pub(in crate::daemon::owner) fn local_moment(&self, at: Timestamp, now: Option<Timestamp>) -> String {
        match now {
            Some(now) if (at.as_utc() - now.as_utc()).abs() < SAME_DAY => self.parts.zone.clock_minute(at),
            Some(_) | None => self.parts.zone.local_minute(at),
        }
    }
}
