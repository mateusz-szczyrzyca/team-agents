//! `agent status`: the teams the daemon knows, their rounds, and what each of their tasks is doing — or, for a round, its
//! item, the tasks that worked for it, its pull request and its review passes.

use std::io::{self, Write};
use std::time::SystemTime;

use agent_core::history::HistoryHealth;
use agent_core::ids::{RoundId, TaskId};
use agent_core::kinds::Role;
use agent_core::local_time::Zone;
use agent_core::plan::judge::JudgementState;
use agent_core::protocol::{
    Client, Command, PlanStepSummary, Reply, RoundDetail, ShowRequest, StatusRequest, Target, TeamSummary,
};
use agent_core::review_findings::Severity;
use agent_core::state::{Round, RoundLife, RunState, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::disk::GIBIBYTE;
use agent_runtime::process::error_chain;
use agent_runtime::run::activity::last_activity;
use agent_runtime::settings::{Settings, SettingsError};
use agent_runtime::store::Home;
use time::UtcOffset;

use crate::client::{ClientError, Connection};

/// Why `agent status` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StatusError {
    /// The daemon could not be asked.
    #[error("cannot ask the daemon for the status")]
    Client(#[from] ClientError),
    /// The daemon answered with something else than a list of teams.
    #[error("the daemon answered a status request with {kind}")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
    },
    /// The settings of the home, which name the investigators, cannot be read.
    #[error("cannot read the settings of the home")]
    Settings(#[source] SettingsError),
    /// The output cannot be written.
    #[error("cannot write the status")]
    Output(#[source] io::Error),
    /// The teams cannot be written as JSON.
    #[error("cannot write the status as JSON")]
    Json(#[source] serde_json::Error),
}

/// Where `agent status` reads when a task that runs was last active, and the zone it tells the time in.
#[derive(Debug, Clone)]
pub(crate) struct Activity {
    /// The home whose runs are looked at.
    home: Home,
    /// The zone the times are written in.
    zone: Zone,
}

impl Activity {
    /// The activity of the tasks of `home`, told in the local time `zone` gives.
    pub(crate) const fn new(home: Home, zone: Zone) -> Self {
        Self { home, zone }
    }

    /// What the line of the running task `task` says of its last activity: the time the output of its runs last changed,
    /// or that it cannot be told; `None` when its runs have written nothing yet.
    fn of(&self, task: &TaskId) -> Option<String> {
        match last_activity(&self.home, task) {
            Ok(Some(changed)) => Some(format!("last activity {}", clock_time(changed, self.zone))),
            Ok(None) => None,
            Err(error) => Some(format!("last activity unknown ({})", error_chain(&error))),
        }
    }

    /// The line about low free disk space when below threshold, or the reason the space or the limit cannot be read.
    /// `None` before the daemon has measured the space, and when the space is above the limit.
    pub(crate) fn disk_line(&self) -> Option<String> {
        let content = match std::fs::read_to_string(self.home.free_space_file()) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => return Some(format!("disk: cannot read the free space ({})\n", error_chain(&error))),
        };
        let free_bytes = match content.trim().parse::<u64>() {
            Ok(bytes) => bytes,
            Err(error) => return Some(format!("disk: cannot read the free space ({error})\n")),
        };
        let low_free_gb = match self.low_free_gb() {
            Ok(gb) => gb,
            Err(message) => return Some(message),
        };
        let free_gb = free_bytes / GIBIBYTE;
        if free_gb < low_free_gb {
            Some(format!("disk: {free_gb} GB free, limit {low_free_gb} GB\n"))
        } else {
            None
        }
    }

    /// The `disk.low_free_gb` of the configuration: the default when the file is absent, the message to show when it
    /// cannot be read or parsed.
    fn low_free_gb(&self) -> Result<u64, String> {
        match std::fs::read_to_string(self.home.config_file()) {
            Ok(text) => agent_core::config::Config::from_toml_str(&text)
                .map(|config| config.disk().low_free_gb)
                .map_err(|error| format!("disk: cannot read the configuration ({})\n", error_chain(&error))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(agent_core::config::Disk::default().low_free_gb)
            }
            Err(error) => Err(format!(
                "disk: cannot read the configuration ({})\n",
                error_chain(&error)
            )),
        }
    }
}

/// `at` as a person reads it in `zone`: the date, the time with its seconds and the zone.
pub(crate) fn clock_time(at: SystemTime, zone: Zone) -> String {
    let utc = time::UtcDateTime::from(at);
    match Timestamp::from_utc(utc) {
        Ok(at) => zone.stamp(at),
        // A time the state cannot hold (outside the years 0 to 9999) is shown in UTC and marked as UTC.
        Err(
            agent_core::state::InvalidTimestamp::YearOutOfRange { .. }
            | agent_core::state::InvalidTimestamp::NotRfc3339 { .. },
        ) => {
            let year = utc.year();
            let year_text = if year < 0 {
                format!("-{:04}", year.unsigned_abs())
            } else {
                format!("{year:04}")
            };
            format!(
                "{year_text}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                u8::from(utc.month()),
                utc.day(),
                utc.hour(),
                utc.minute(),
                utc.second(),
            )
        }
    }
}

/// `at` as a person reads it in `zone`: the time of a recorded activity.
pub(crate) fn clock_of(at: Timestamp, _zone: Zone) -> String {
    Zone::Utc.stamp(at)
}

/// Asks the daemon for the status of `target` (every team without one) and writes it to `out`; the tasks that run say
/// when they were last active.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_active(
    connection: &mut Connection,
    target: Option<Target>,
    activity: &Activity,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    if let Some(Target::Round(round)) = target {
        return show_round(connection, round, Some(activity), out).await;
    }
    let (teams, history) = teams(connection, target).await?;
    let told = rounds::told(connection, activity, &teams).await?;
    let text = render_at(
        &teams,
        &|task| activity.of(task),
        &|round| told.of(round),
        activity.zone,
    );
    write(
        out,
        &format!("{text}{}", told.unread()),
        history.as_ref(),
        activity.disk_line().as_deref(),
    )
}

/// [`status_active`] without the last activity of the tasks: what the tests of the status ask for.
#[cfg(test)]
pub(crate) async fn status(
    connection: &mut Connection,
    target: Option<Target>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    if let Some(Target::Round(round)) = target {
        return show_round(connection, round, None, out).await;
    }
    let (teams, history) = teams(connection, target).await?;
    write(out, &render(&teams), history.as_ref(), None)
}

/// Asks the daemon for every team and writes to `out` what `investigators::render` says of the investigators configured in
/// `home`: for each of them its last two activities in the teams that are not closed, and the one that has none as free.
///
/// # Errors
/// [`StatusError`]: the settings of `home` cannot be read, the daemon cannot be asked or answers with something else
/// than a list of teams, or the output cannot be written.
pub(crate) async fn status_investigators(
    connection: &mut Connection,
    home: &Home,
    zone: Zone,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let settings = Settings::load(home).await.map_err(StatusError::Settings)?;
    // The configuration keeps the investigators by name: the view lists them in that order.
    let configured: Vec<_> = settings.config.investigators().keys().cloned().collect();
    let (teams, _) = teams(connection, None).await?;
    let text = investigators::render(&teams, &configured, zone);
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// Asks the daemon for every team and writes to `out` those that have a round that goes on, with those rounds only:
/// what is being worked on now.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_running(
    connection: &mut Connection,
    activity: &Activity,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let (teams, history) = teams(connection, None).await?;
    // A team is listed when it has a round that goes on; the parts it prints come from the whole team, since a merged
    // part's round is one that is over.
    let listed: Vec<TeamSummary> = teams
        .into_iter()
        .filter(|team| team.rounds.iter().any(|round| *round.life() == RoundLife::Open))
        .collect();
    let text = if listed.is_empty() {
        NO_OPEN_ROUNDS.to_owned()
    } else {
        let told = rounds::told(connection, activity, &listed).await?;
        let text = render_in(
            &listed,
            View::Running,
            &|task| activity.of(task),
            &|round| told.of(round),
            activity.zone,
        );
        format!("{text}{}", told.unread())
    };
    write(out, &text, history.as_ref(), activity.disk_line().as_deref())
}

/// The teams of `teams` that have an open round, each with its open rounds only.
#[cfg(test)]
pub(crate) fn running(teams: Vec<TeamSummary>) -> Vec<TeamSummary> {
    teams
        .into_iter()
        .filter_map(|mut team| {
            team.rounds.retain(|round| *round.life() == RoundLife::Open);
            (!team.rounds.is_empty()).then_some(team)
        })
        .collect()
}

/// Asks the daemon for the status of `target` (every team without one) and writes the teams to `out` as one JSON array,
/// each team with its description.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_json(
    connection: &mut Connection,
    target: Option<Target>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let (teams, _) = teams(connection, target).await?;
    out.write_all(render_json(&teams)?.as_bytes())
        .map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// The teams as one JSON array and a newline.
///
/// # Errors
/// [`StatusError::Json`] when the teams cannot be written as JSON.
pub(crate) fn render_json(teams: &[TeamSummary]) -> Result<String, StatusError> {
    let mut text = serde_json::to_string(teams).map_err(StatusError::Json)?;
    text.push('\n');
    Ok(text)
}

/// The teams the daemon answers a status request for `target` with, and what it says of the history.
async fn teams(
    connection: &mut Connection,
    target: Option<Target>,
) -> Result<(Vec<TeamSummary>, Option<HistoryHealth>), StatusError> {
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target }))
        .await?;
    match reply {
        Reply::Teams { teams } => Ok((teams, None)),
        Reply::Overview { teams, history } => Ok((teams, Some(history))),
        other => Err(StatusError::UnexpectedReply { kind: kind_of(&other) }),
    }
}

/// Writes `text`, then the line about low disk space if any, then the line about the history when it is not whole.
fn write(
    out: &mut impl Write,
    text: &str,
    history: Option<&HistoryHealth>,
    disk: Option<&str>,
) -> Result<(), StatusError> {
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    if let Some(disk) = disk {
        out.write_all(disk.as_bytes()).map_err(StatusError::Output)?;
    }
    out.write_all(render_history(history).as_bytes())
        .map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// Asks the daemon to show the round `round`.
async fn round_detail(connection: &mut Connection, round: RoundId) -> Result<RoundDetail, StatusError> {
    let reply = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Round(round),
            }),
        )
        .await?;
    match reply {
        Reply::Round { round } => Ok(*round),
        other => Err(StatusError::UnexpectedReply { kind: kind_of(&other) }),
    }
}

/// Asks the daemon to show the round `round` and writes it to `out`.
async fn show_round(
    connection: &mut Connection,
    round: RoundId,
    activity: Option<&Activity>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let detail = round_detail(connection, round.clone()).await?;
    let zone = activity.map_or(Zone::Fixed(UtcOffset::UTC), |activity| activity.zone);
    let mut text = render_round_in(&detail, zone);
    if let Some(activity) = activity {
        text.push_str(&snapshots::snapshots_of(&activity.home, &round, activity.zone));
    }
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// The name of a kind of reply, for an error message.
pub(super) const fn kind_of(reply: &Reply) -> &'static str {
    match reply {
        Reply::Started { .. } => "started",
        Reply::Teams { .. } => "teams",
        Reply::Overview { .. } => "overview",
        Reply::Team { .. } => "team",
        Reply::Round { .. } => "round",
        Reply::Task { .. } => "task",
        Reply::Applied { .. } => "applied",
        Reply::Gate { .. } => "gate",
        Reply::GateRecorded { .. } => "gate_recorded",
        Reply::Subscribed { .. } => "subscribed",
    }
}

/// The teams as text: a line for each team, and under it a line for each of its tasks, without the last activity of the
/// tasks: what the tests of the layout compare.
#[cfg(test)]
pub(crate) fn render(teams: &[TeamSummary]) -> String {
    render_active(teams, &|_| None)
}

/// The teams as text: a line for each team, and under it a line for each of its tasks, with what `activity` says of each
/// task that runs after its status; the rounds say nothing beyond their lines.
pub(crate) fn render_active(teams: &[TeamSummary], activity: &dyn Fn(&TaskId) -> Option<String>) -> String {
    render_at(teams, activity, &|_| String::new(), Zone::Fixed(UtcOffset::UTC))
}

/// [`render_active`] with what `rounds` says of each round after its line, the times in UTC: what the tests of the
/// rounds compare.
#[cfg(test)]
fn render_with(
    teams: &[TeamSummary],
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
) -> String {
    render_at(teams, activity, rounds, Zone::Fixed(UtcOffset::UTC))
}

/// [`render_active`] with what `rounds` says of each round after its line, the times of the recorded activities told in
/// `zone`.
fn render_at(
    teams: &[TeamSummary],
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    zone: Zone,
) -> String {
    render_in(teams, View::Full, activity, rounds, zone)
}

/// What a view of the status prints, the running view being the same teams with their open rounds only.
#[cfg(test)]
pub(crate) fn render_running(teams: &[TeamSummary]) -> String {
    render_in(
        teams,
        View::Running,
        &|_| None,
        &|_| String::new(),
        Zone::Fixed(UtcOffset::UTC),
    )
}

/// Which view of the status is printed: the full one, or the running one that limits the round lines to the open ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    /// Every round of a team.
    Full,
    /// The open rounds only.
    Running,
}

fn render_in(
    teams: &[TeamSummary],
    view: View,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    zone: Zone,
) -> String {
    if teams.is_empty() {
        return NO_TEAMS.to_owned();
    }
    let members = || teams.iter().flat_map(|team| team.members.iter());
    let role_width = members().map(|member| member.role.to_string().len()).max().unwrap_or(0);
    let agent_width = members().map(|member| member.agent.name().len()).max().unwrap_or(0);
    let mut lines: Vec<String> = Vec::new();
    for team in teams {
        let items = &team.items;
        let task = team
            .short_desc
            .as_ref()
            .map_or_else(String::new, |desc| format!("  \"{desc}\""));
        lines.push(format!(
            "{}  {}  {}  {} pending, {} in progress, {} in review, {} approved, {} done{task}",
            team.id,
            team.repo,
            team.merge.as_str(),
            items.pending,
            items.in_progress,
            items.in_review,
            items.approved,
            items.done,
        ));
        lines.extend(
            team.rounds
                .iter()
                .filter(|round| view == View::Full || *round.life() == RoundLife::Open)
                .map(|round| format!("  {}{}", round_line(round), rounds(round))),
        );
        lines.extend(splits::lines_of(team, view));
        for (plan, _) in splits::shown(team, view) {
            lines.extend(
                crate::split_context::lines(plan, &team.rounds, &team.part_assignments)
                    .into_iter()
                    .map(|(_, text)| format!("      {text}")),
            );
        }
        for (item, tier) in &team.tier_waits {
            if team.splits.iter().any(|plan| plan.parent() == *item) {
                continue;
            }
            lines.push(format!("    {item}  waiting for {tier} agent"));
        }
        for member in &team.members {
            let waiting = member.open_question.map_or_else(String::new, |number| {
                format!(", questions number {number} wait for answers")
            });
            let renewed = member
                .new_sessions
                .map_or_else(String::new, |_| ", in a new session".to_owned());
            let taken_over = member
                .held_by
                .as_ref()
                .map_or_else(String::new, |held| format!(", for {held}"));
            // Only a task that runs is active now: what a task that waits last did is in its log.
            let active = (member.status == TaskStatus::Running)
                .then(|| activity(&member.task))
                .flatten()
                .map_or_else(String::new, |said| format!(", {said}"));
            let status = match member.role {
                Role::Reviewer => {
                    reviewer::status(member, &team.activities, zone).unwrap_or_else(|| member.status.to_string())
                }
                Role::Solo | Role::Coordinator | Role::Member | Role::Worker => member.status.to_string(),
            };
            lines.push(format!(
                "  {}  {:<role_width$}  {:<agent_width$}  {status}{renewed}{taken_over}{active}{waiting}",
                member.task,
                member.role.to_string(),
                member.agent.name(),
            ));
            if let Some(steps) = team.task_plans.get(&member.task) {
                for step in steps {
                    lines.push(render_plan_step(step));
                }
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The line `agent status` shows about the history when it is not whole: what it is doing and how many records it lost.
pub(crate) fn render_history(history: Option<&HistoryHealth>) -> String {
    history.map_or_else(String::new, |health| format!("{health}\n"))
}

/// A round as one line: its ID, its item and whether it is open or how it ended.
pub(crate) fn round_line(round: &Round) -> String {
    let item = round
        .item()
        .map_or_else(|| "whole task list".to_owned(), |item| format!("item {item}"));
    let state = match (round.delivered_by(), round.life()) {
        (Some(pr), _) if round.delivery_closed_at().is_some() => {
            format!("closed, abandoned, delivery PR #{} closed without merging", pr.number)
        }
        (Some(pr), RoundLife::Open) => format!("delivered by PR #{}, waits for its merge", pr.number),
        (
            Some(pr),
            RoundLife::Closed {
                how: agent_core::state::RoundEnd::Merged,
                ..
            },
        ) => format!("done, delivered by PR #{}", pr.number),
        (
            Some(pr),
            RoundLife::Closed {
                how: agent_core::state::RoundEnd::Abandoned,
                ..
            },
        ) => format!("closed, abandoned, delivered by PR #{}", pr.number),
        (Some(_) | None, life) => match life {
            RoundLife::Open => "open".to_owned(),
            RoundLife::Closed { how, .. } => format!("closed, {how}"),
        },
    };
    format!("{}  {item}  {state}", round.id())
}

/// A round in full as text, its times in UTC: what the tests of the layout compare.
#[cfg(test)]
pub(crate) fn render_round(detail: &RoundDetail) -> String {
    render_round_in(detail, Zone::Fixed(UtcOffset::UTC))
}

/// A round in full as text: its line, the item, the pull request, the review passes, and the tasks that worked for it
/// with where they work and their runs, its times in `zone`.
pub(crate) fn render_round_in(detail: &RoundDetail, zone: Zone) -> String {
    let waiting = detail.waiting.as_deref().map_or_else(String::new, |waiting| {
        format!(", waits for {}", zone.stamp_waiting(waiting))
    });
    let mut lines = vec![format!("{}  team {}{waiting}", round_line(&detail.round), detail.team)];
    lines.push(format!("  opened {}", zone.stamp(detail.round.opened_at())));
    if let RoundLife::Closed { at, how } = detail.round.life() {
        lines.push(format!("  closed {}, {how}", zone.stamp(*at)));
    }
    if let Some(item) = &detail.item {
        lines.push(format!("  item {}: {} ({})", item.id, item.title, item.status));
    }
    if let Some(pr) = &detail.pr {
        lines.push(format!("  pull request #{} on {}", pr.number, pr.branch));
    }
    if !detail.findings.is_empty() {
        let count = |severity| {
            detail
                .findings
                .iter()
                .filter(|entry| {
                    entry.status == agent_core::state::FindingStatus::Open && entry.finding.severity == severity
                })
                .count()
        };
        lines.push(format!(
            "  open findings: {} BLOCKER, {} MAJOR, {} MINOR",
            count(Severity::Blocker),
            count(Severity::Major),
            count(Severity::Minor)
        ));
    }
    for (index, pass) in detail.review_passes.iter().enumerate() {
        let reviews: Vec<String> = pass
            .reviews
            .iter()
            .map(|review| match &review.outcome {
                agent_core::state::ReviewOutcome::Pending => format!("{} pending", review.reviewer),
                agent_core::state::ReviewOutcome::Submitted { verdict, .. } => {
                    format!("{} {verdict}", review.reviewer)
                }
            })
            .collect();
        lines.push(format!(
            "  review pass {}: {}",
            index.saturating_add(1),
            reviews.join(", ")
        ));
    }
    for task in &detail.tasks {
        lines.push(format!(
            "  {}  {}  {}  {}  {}",
            task.task,
            task.role,
            task.agent.name(),
            task.worktree.as_path().display(),
            task.branch
        ));
        for run in &task.runs {
            let state = match run.state() {
                RunState::Running => "running".to_owned(),
                RunState::Ended { at, .. } => format!("ended {}", zone.stamp(*at)),
            };
            lines.push(format!(
                "    {}  started {}  {state}",
                run.id(),
                zone.stamp(run.started_at())
            ));
        }
        if let Some(steps) = detail.task_plans.get(&task.task) {
            for step in steps {
                lines.push(render_plan_step(step));
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// A step of a plan as a line of a task: its mark, its ID, its title, and its settled state when judged.
pub(crate) fn render_plan_step(step: &PlanStepSummary) -> String {
    let state_suffix = match step.state {
        Some(JudgementState::Confirmed) => " (confirmed)".to_owned(),
        Some(JudgementState::Declared) => " (declared)".to_owned(),
        Some(JudgementState::Mismatch) => match &step.reason {
            Some(reason) => format!(" (mismatch: {reason})"),
            None => " (mismatch)".to_owned(),
        },
        None => String::new(),
    };
    format!("    [{}] {} {}{}", step.mark.mark(), step.id, step.title, state_suffix)
}

/// What is written when the daemon knows no team.
const NO_TEAMS: &str = "no teams\n";

/// What `agent status --running` writes when no round goes on.
const NO_OPEN_ROUNDS: &str = "no team has an open round\n";

mod investigators;
mod reviewer;
mod rounds;
mod snapshots;
mod splits;

#[cfg(test)]
#[path = "status/history/tests.rs"]
mod history_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "status/tests/splits.rs"]
mod split_tests;

#[cfg(test)]
#[path = "status/tests/negotiation.rs"]
mod negotiation_tests;

#[cfg(test)]
#[path = "status/tests/short_desc.rs"]
mod short_desc_tests;

#[cfg(test)]
#[path = "status/tests/part_states.rs"]
mod part_state_tests;
