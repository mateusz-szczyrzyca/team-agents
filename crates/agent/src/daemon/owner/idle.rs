//! Automatic idle removal uses the same safeguards as the human command.
use super::{Owner, OwnerError};
use agent_core::decide::idle::{Decision, Hold, decide};
use agent_core::ids::TeamId;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::protocol::{Client, Failure, RemoveRequest};
use agent_core::state::{IdleWatch, RunState, Team, Timestamp};
use agent_core::status::TaskStatus;
use std::collections::BTreeMap;

impl Owner {
    /// Observes activity and attempts removal of teams beyond their idle limit.
    pub(super) async fn check_idle_teams(&mut self) -> Result<(), OwnerError> {
        let limit = self.parts.settings.config.limits().idle_team_minutes.as_duration();
        if limit.is_zero() {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let teams: Vec<_> = self.parts.state.teams().iter().map(|team| team.id().clone()).collect();
        for id in teams {
            let Some(team) = self.parts.state.team(&id) else {
                continue;
            };
            let hold = self.idle_hold(team, now);
            let recorded = if hold == Hold::Running {
                now
            } else {
                recorded_activity(team)
            };
            let watch = IdleWatch::observe(team.idle_watch(), recorded, now, BTreeMap::new());
            let decision = decide(now, watch.last_activity, limit, hold);
            let reason = format!("idle since {} (limit {} seconds)", watch.last_activity, limit.as_secs());
            if let Some(mut team) = self.parts.state.team_mut(&id) {
                team.record_idle_watch(watch);
            }
            self.save().await?;
            if decision == Decision::Remove {
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
    fn idle_hold(&self, team: &Team, now: Timestamp) -> Hold {
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
                return Hold::Running;
            }
            match member.status() {
                TaskStatus::Running => return Hold::Running,
                TaskStatus::Paused | TaskStatus::Stopped => return Hold::Human,
                TaskStatus::Queued
                | TaskStatus::Waiting
                | TaskStatus::Questions
                | TaskStatus::Failed
                | TaskStatus::Done => {}
            }
            if self.wakes.get(task).is_some_and(|wake| *wake > now) {
                return Hold::Provider;
            }
        }
        Hold::None
    }

    /// Persist the first refusal before notifying, so a restart cannot repeat it.
    async fn idle_refusal(&mut self, id: &TeamId, failure: Failure) -> Result<(), OwnerError> {
        let Some(watch) = self.parts.state.team(id).and_then(Team::idle_watch) else {
            return Ok(());
        };
        if watch.refusal.is_some() {
            return Ok(());
        }
        let mut watch = watch.clone();
        watch.refusal = Some(failure.message.clone());
        if let Some(mut team) = self.parts.state.team_mut(id) {
            team.record_idle_watch(watch);
        }
        self.save().await?;
        let rendered = Variables::new()
            .with("team", id.to_string())
            .and_then(|v| v.with("reason", &failure.message))
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
                tracing::warn!(event = agent_core::log::LogEvent::NotificationFailed.as_str(), team = %id, error = %error.message, "cannot render idle refusal")
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
        .flat_map(|member| member.runs())
        .fold(team.created_at(), |last, run| {
            last.max(run.started_at()).max(match run.state() {
                RunState::Running => run.started_at(),
                RunState::Ended { at, .. } => *at,
            })
        })
}
#[cfg(test)]
mod tests;
