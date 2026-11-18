//! A team that sleeps with its work unfinished: its lead is woken and told that nobody works, and when that does not
//! help the human is told.

use agent_core::decide::asleep::{AsleepStep, BlockedItemStep, Team, asleep_step, blocked_item_step};
use agent_core::ids::TeamId;
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::state::{ItemPhase, Member, RunEnd, RunState, Timestamp};
use agent_core::status::TaskStatus;

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::{Announcement, AutopilotNotice, Lead, PublishError, Pulls};
use crate::daemon::team::Why;

/// How many times in a row the lead of a team is woken because the team sleeps; after that the human is told.
pub(super) const ASLEEP_WAKES: u32 = 2;

/// What a look at a team found, kept apart from the state so that the owner can change it.
struct Found {
    /// The numbers of the items that are in progress or being fixed.
    unfinished: Vec<String>,
    /// The status of every task of the crew, the lead first.
    statuses: Vec<TaskStatus>,
    /// How every task of the crew ended, a line each.
    tasks: Vec<String>,
    /// When the last run of the crew ended; `None` while one runs or none has run.
    quiet_since: Option<Timestamp>,
    /// Standing items: items in progress whose assignee is failed.
    standings: Vec<Standing>,
}

/// A standing item in progress whose assignee is failed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Standing {
    /// The item id.
    item: String,
    /// The member agent name.
    member: String,
    /// The member task id.
    task: agent_core::ids::TaskId,
    /// The member status and last run detail line.
    line: String,
    /// The current status of the assignee task.
    assignee_status: TaskStatus,
}

/// How the task of `member` ended, in a line for the lead and the human.
fn line(role: Role, member: &Member) -> String {
    let how = match member.runs().last().map(agent_core::state::Run::state) {
        Some(RunState::Ended { at, end }) => {
            let end = match end {
                RunEnd::Reported { outcome } => format!("STATUS: {outcome}"),
                RunEnd::NoStatus { .. } => "no STATUS line".to_owned(),
                RunEnd::QuotaExhausted { .. } => "the quota was used up".to_owned(),
                RunEnd::RateLimited => "the provider limited the rate".to_owned(),
                RunEnd::Hung => "it hung".to_owned(),
                RunEnd::TurnLimit => "it reached its turn limit".to_owned(),
                RunEnd::Stopped => "it was stopped".to_owned(),
            };
            format!("its last run ended at {at} with: {end}")
        }
        Some(RunState::Running) => "a run of it is going".to_owned(),
        None => "it has not run".to_owned(),
    };
    format!(
        "- {} ({role}, task {}): {}; {how}",
        member.agent().name(),
        member.task(),
        member.status()
    )
}

impl Owner {
    /// What the state says of the team `team_id`, or `None` when it is gone.
    fn look_at_sleep(&self, team_id: &TeamId) -> Option<Found> {
        let team = self.parts.state.team(team_id)?;
        let roster = team.crew().roster();
        let unfinished = team
            .items()
            .iter()
            .filter(|progress| {
                matches!(
                    progress.phase(),
                    ItemPhase::InProgress { .. } | ItemPhase::Fixing { .. }
                )
            })
            .map(|progress| progress.id().to_string())
            .collect();
        let ended: Option<Vec<Timestamp>> = roster
            .iter()
            .filter_map(|(_, member)| member.runs().last())
            .map(|run| match run.state() {
                RunState::Ended { at, .. } => Some(*at),
                RunState::Running => None,
            })
            .collect();
        let standings: Vec<Standing> = team
            .items()
            .iter()
            .filter_map(|progress| {
                let (ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. }) = progress.phase()
                else {
                    return None;
                };
                let (role, member) = roster.iter().find(|(_, m)| m.agent().name() == assignee.as_str())?;
                if member.status() == TaskStatus::Failed {
                    Some(Standing {
                        item: progress.id().to_string(),
                        member: assignee.to_string(),
                        task: member.task().clone(),
                        line: line(*role, member),
                        assignee_status: member.status(),
                    })
                } else {
                    None
                }
            })
            .collect();
        Some(Found {
            unfinished,
            statuses: roster.iter().map(|(_, member)| member.status()).collect(),
            tasks: roster.iter().map(|(role, member)| line(*role, member)).collect(),
            quiet_since: ended.and_then(|times| times.into_iter().max()),
            standings,
        })
    }

    /// Checks standing items behind failed members, resets dedup when assignee leaves failed, and notifies human if lead is done.
    fn check_standing_items(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        found: &Found,
    ) -> Result<(), OwnerError> {
        if let Some(team) = self.parts.state.team(team_id) {
            for item in team.items() {
                let item_id = item.id().to_string();
                if !found.standings.iter().any(|s| s.item == item_id) {
                    self.watch.announced.remove(&Announcement::ItemBlocked {
                        team: team_id.clone(),
                        item: item_id,
                    });
                }
            }
        }

        let lead_status = found.statuses.first().copied().unwrap_or(TaskStatus::Waiting);
        let now = Timestamp::now(&self.parts.clock)?;
        for standing in &found.standings {
            if blocked_item_step(lead.done, lead_status, standing.assignee_status) == BlockedItemStep::TellHuman {
                let announcement = Announcement::ItemBlocked {
                    team: team_id.clone(),
                    item: standing.item.clone(),
                };
                if self.watch.announced.insert(announcement) {
                    let values = Variables::new()
                        .with("team", team_id.as_str())
                        .and_then(|v| v.with("item", standing.item.as_str()))
                        .and_then(|v| v.with("member", standing.member.as_str()))
                        .and_then(|v| v.with("task", standing.task.as_str()))
                        .and_then(|v| v.with("line", standing.line.as_str()));
                    self.tell_human(pulls, AutopilotNotice::StandingItem, values, lead, now);
                }
            }
        }
        Ok(())
    }

    /// Wakes the lead of the team `team_id` when every task of it sleeps and an item is unfinished, or tells the human
    /// when the lead was woken for it as often as it is.
    pub(super) fn wake_sleeping_team(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Some(found) = self.look_at_sleep(team_id) else {
            return Ok(());
        };

        let lead = self.lead_of(team_id)?;
        if lead.done {
            // The lead handed its branch over: what follows is a new stretch of work.
            self.watch.asleep_wakes.remove(team_id);
        }

        self.check_standing_items(pulls, team_id, &lead, &found)?;

        let Some(quiet_since) = found.quiet_since else {
            return Ok(());
        };
        let now = Timestamp::now(&self.parts.clock)?;
        let quiet_for = std::time::Duration::try_from(now.as_utc() - quiet_since.as_utc()).unwrap_or_default();
        let woken = self.watch.asleep_wakes.get(team_id).copied().unwrap_or(0);
        let step = asleep_step(&Team {
            unfinished: !found.unfinished.is_empty(),
            tasks: &found.statuses,
            lead_done: lead.done,
            standing_item: !found.standings.is_empty(),
            quiet_for,
            may_sleep: self.parts.settings.config.watchdog().asleep_minutes.as_duration(),
            woken,
            wake_limit: ASLEEP_WAKES,
        });
        let values = |woken: u32| {
            Variables::new()
                .with("team", team_id.as_str())
                .and_then(|values| values.with("items", found.unfinished.join(", ")))
                .and_then(|values| values.with("minutes", (quiet_for.as_secs() / 60).to_string()))
                .and_then(|values| values.with("tasks", found.tasks.join("\n")))
                .and_then(|values| values.with("woken", woken.to_string()))
        };
        match step {
            AsleepStep::Nothing => Ok(()),
            AsleepStep::WakeLead => {
                let woken = woken.saturating_add(1);
                let text = values(woken)
                    .and_then(|values| values.with("limit", ASLEEP_WAKES.to_string()))
                    .map_err(PublishError::from)
                    .and_then(|values| Ok(pulls.texts.render(AutopilotPrompt::TeamAsleep, &values)?));
                let text = match text {
                    Ok(text) => text,
                    Err(error) => {
                        tracing::error!(
                            event = LogEvent::NotificationFailed.as_str(),
                            team = %team_id,
                            error = %agent_runtime::process::error_chain(&error),
                            "what a lead is told about its sleeping team cannot be made"
                        );
                        return Ok(());
                    }
                };
                tracing::info!(
                    event = LogEvent::ChecksDecided.as_str(),
                    outcome = "team_asleep",
                    team = %team_id,
                    task = %lead.task,
                    woken,
                    "every task of a team sleeps with an item unfinished: its lead is woken"
                );
                self.watch.asleep_wakes.insert(team_id.clone(), woken);
                self.release(&lead.task, Why::Asleep(text.trim_end().to_owned()), look)
            }
            AsleepStep::TellHuman => {
                if lead.done {
                    return Ok(());
                }
                let announcement = Announcement::TeamAsleep {
                    team: team_id.clone(),
                    since: quiet_since,
                };
                if self.watch.announced.insert(announcement) {
                    let values = values(woken).and_then(|values| values.with("lead", lead.task.as_str()));
                    self.tell_human(pulls, AutopilotNotice::TeamAsleep, values, &lead, now);
                }
                Ok(())
            }
        }
    }
}
