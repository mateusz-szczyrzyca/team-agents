//! Automatic idle removal uses the same safeguards as the human command.
use super::{Owner, OwnerError};
use agent_core::decide::idle::{Decision, Hold, decide};
use agent_core::decide::wait::Obligation;
use agent_core::ids::TeamId;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::protocol::{Client, Failure, RemoveRequest};
use agent_core::state::{IdleWatch, RoundLife, RunState, Team, Timestamp};
use agent_core::status::TaskStatus;
use std::collections::BTreeMap;
mod activity;
mod journal;
use journal::Outcome;

impl Owner {
    /// Observes activity and attempts removal of teams beyond their idle limit.
    pub(super) async fn check_idle_teams(&mut self) -> Result<(), OwnerError> {
        let limit = self.parts.settings.config.limits().idle_team_minutes.as_duration();
        if limit.is_zero() {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let started_at = Timestamp::from_utc(self.started_at)?;
        let teams: Vec<_> = self.parts.state.teams().iter().map(|team| team.id().clone()).collect();
        for id in teams {
            let Some(team) = self.parts.state.team(&id) else {
                continue;
            };
            let observation = self.observe_idle(team, now).await;
            let (watch, hold) = match observation {
                Ok(observation) => observation,
                Err(failure) => {
                    if team.idle_watch().is_none() {
                        let watch = IdleWatch::observe(None, recorded_activity(team), now, BTreeMap::new());
                        if let Some(mut team) = self.parts.state.team_mut(&id) {
                            team.record_idle_watch(watch);
                        }
                    }
                    self.idle_refusal(&id, failure).await?;
                    continue;
                }
            };
            let idle_since = watch.last_activity.max(started_at);
            let decision = decide(now, idle_since, limit, hold);
            let reason = format!("idle since {idle_since} (limit {} seconds)", limit.as_secs());
            if let Some(mut team) = self.parts.state.team_mut(&id) {
                team.record_idle_watch(watch);
            }
            self.save().await?;
            if decision == Decision::Remove {
                let first_attempt = self
                    .parts
                    .state
                    .team(&id)
                    .and_then(Team::idle_watch)
                    .is_some_and(|watch| watch.refusal.is_none());
                if first_attempt && let Err(failure) = self.journal_idle(&id, Outcome::Attempt, &reason).await {
                    self.idle_refusal(&id, failure).await?;
                    continue;
                }
                if !first_attempt {
                    continue;
                }
                let request = RemoveRequest {
                    team: id.clone(),
                    force: false,
                };
                if let Err(failure) = self.remove_for(&Client::Cli, request, &reason).await? {
                    self.idle_refusal(&id, failure).await?;
                }
            }
        }
        Ok(())
    }

    /// Activity and intentional holds prevent an automatic removal attempt.
    fn idle_hold(&self, team: &Team, now: Timestamp) -> Result<Hold, Failure> {
        if team.delivery_waits().next().is_some() {
            return Ok(Hold::ExternalMerge);
        }
        if team.rounds().iter().any(|round| {
            *round.life() == RoundLife::Open && self.round_obligation(team, round) == Some(Obligation::FreeInvestigator)
        }) {
            return Ok(Hold::Queued);
        }
        for member in team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers())
        {
            let task = member.task();
            if self.in_flight.contains(task)
                || self.parked.contains_key(task)
                || self.request_checking.values().any(|checking| checking.task() == task)
                || self.gate_answer_tasks.values().any(|owner| owner == task)
            {
                return Ok(Hold::Running);
            }
            match member.status() {
                TaskStatus::Running => return Ok(Hold::Running),
                TaskStatus::Paused | TaskStatus::Stopped => return Ok(Hold::Human),
                TaskStatus::Queued => return Ok(Hold::Queued),
                TaskStatus::Waiting | TaskStatus::Questions | TaskStatus::Failed | TaskStatus::Done => {}
            }
            if let Some(run) = member.runs().last()
                && let RunState::Ended { at, end } = run.state()
                && super::super::settle::wake_time(end, *at, self.parts.settings.config.limits())
                    .map_err(|error| super::removal::internal("cannot reconstruct provider wait", &error))?
                    .is_some_and(|wake| wake > now || member.status() == TaskStatus::Queued)
            {
                // A due provider retry stays protected until the scheduler begins its next run.
                return Ok(Hold::Provider);
            }
            if self.wakes.get(task).is_some_and(|wake| *wake > now) {
                return Ok(Hold::Provider);
            }
        }
        Ok(Hold::None)
    }

    /// Persist the first refusal before notifying, so a restart cannot repeat it.
    async fn idle_refusal(&mut self, id: &TeamId, failure: Failure) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(id) else {
            return Ok(());
        };
        let Some(watch) = team.idle_watch() else {
            return Ok(());
        };
        if watch.refusal.is_some() {
            return Ok(());
        }
        let mut watch = watch.clone();
        let branches = super::removal::branches_of(team);
        let message = match branches {
            Ok(branches) => format!(
                "{}; branches: {}",
                failure.message,
                branches.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            ),
            Err(error) => format!("{}; cannot list branches: {}", failure.message, error.message),
        };
        if let Err(error) = self.journal_idle(id, Outcome::Refused, &message).await {
            // The team remains intact; report journal failure while retaining the refusal in state.
            tracing::warn!(team = %id, error = %error.message, "cannot persist idle refusal history");
        }
        watch.refusal = Some(message.clone());
        if let Some(mut team) = self.parts.state.team_mut(id) {
            team.record_idle_watch(watch);
        }
        self.save().await?;
        let rendered = Variables::new()
            .with("team", id.to_string())
            .and_then(|v| v.with("reason", &message))
            .map_err(|error| super::removal::internal("cannot build idle refusal notice", &error))
            .and_then(|values| {
                self.parts
                    .texts
                    .render(HarnessPrompt::NoticeIdleRefused, &values)
                    .map_err(|error| super::removal::internal("cannot render idle refusal notice", &error))
            });
        match rendered {
            Ok(text) => {
                let (title, body) = text.split_once('\n').unwrap_or((text.as_str(), ""));
                self.deliver(title, body, "idle removal refusal").await;
            }
            Err(error) => {
                tracing::warn!(event = agent_core::log::LogEvent::NotificationFailed.as_str(), team = %id, error = %error.message, "cannot render idle refusal");
            }
        }
        Ok(())
    }
}

/// Existing states reconstruct activity from recorded runs, never daemon startup.
fn recorded_activity(team: &Team) -> Timestamp {
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member)
        .chain(team.reviewers())
        .flat_map(agent_core::state::Member::runs)
        .fold(team.created_at(), |last, run| {
            last.max(run.started_at()).max(match run.state() {
                RunState::Running => run.started_at(),
                RunState::Ended { at, .. } => *at,
            })
        })
}
#[cfg(test)]
mod tests;
