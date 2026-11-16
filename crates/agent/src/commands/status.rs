//! `agent status`: the teams the daemon knows, their rounds, and what each of their tasks is doing — or, for a round, its
//! item, the tasks that worked for it, its pull request and its review passes.

use std::io::{self, Write};

use agent_core::history::HistoryHealth;
use agent_core::ids::RoundId;
use agent_core::protocol::{Client, Command, Reply, RoundDetail, ShowRequest, StatusRequest, Target, TeamSummary};
use agent_core::state::{Round, RoundLife, RunState};

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
    /// The output cannot be written.
    #[error("cannot write the status")]
    Output(#[source] io::Error),
}

/// Asks the daemon for the status of `target` (every team without one) and writes it to `out`.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status(
    connection: &mut Connection,
    target: Option<Target>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    if let Some(Target::Round(round)) = target {
        return show_round(connection, round, out).await;
    }
    let (teams, history) = teams(connection, target).await?;
    write(out, &render(&teams), history.as_ref())
}

/// Asks the daemon for every team and writes to `out` those that have a round that goes on, with those rounds only:
/// what is being worked on now.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_running(connection: &mut Connection, out: &mut impl Write) -> Result<(), StatusError> {
    let (teams, history) = teams(connection, None).await?;
    let teams = running(teams);
    let text = if teams.is_empty() {
        NO_OPEN_ROUNDS.to_owned()
    } else {
        render(&teams)
    };
    write(out, &text, history.as_ref())
}

/// The teams of `teams` that have an open round, each with its open rounds only.
pub(crate) fn running(teams: Vec<TeamSummary>) -> Vec<TeamSummary> {
    teams
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

/// Writes `text`, then the line about the history when it is not whole.
fn write(out: &mut impl Write, text: &str, history: Option<&HistoryHealth>) -> Result<(), StatusError> {
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    out.write_all(render_history(history).as_bytes())
        .map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// Asks the daemon to show the round `round` and writes it to `out`.
async fn show_round(connection: &mut Connection, round: RoundId, out: &mut impl Write) -> Result<(), StatusError> {
    let reply = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Round(round),
            }),
        )
        .await?;
    let Reply::Round { round } = reply else {
        return Err(StatusError::UnexpectedReply { kind: kind_of(&reply) });
    };
    out.write_all(render_round(&round).as_bytes())
        .map_err(StatusError::Output)?;
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

/// The teams as text: a line for each team, and under it a line for each of its tasks.
pub(crate) fn render(teams: &[TeamSummary]) -> String {
    if teams.is_empty() {
        return NO_TEAMS.to_owned();
    }
    let members = || teams.iter().flat_map(|team| team.members.iter());
    let role_width = members().map(|member| member.role.to_string().len()).max().unwrap_or(0);
    let agent_width = members().map(|member| member.agent.name().len()).max().unwrap_or(0);
    let mut lines: Vec<String> = Vec::new();
    for team in teams {
        let items = &team.items;
        lines.push(format!(
            "{}  {}  {}  {} pending, {} in progress, {} in review, {} approved, {} done",
            team.id,
            team.repo,
            team.merge.as_str(),
            items.pending,
            items.in_progress,
            items.in_review,
            items.approved,
            items.done,
        ));
        lines.extend(team.rounds.iter().map(|round| format!("  {}", round_line(round))));
        for member in &team.members {
            let waiting = member.open_question.map_or_else(String::new, |number| {
                format!(", questions number {number} wait for answers")
            });
            lines.push(format!(
                "  {}  {:<role_width$}  {:<agent_width$}  {}{waiting}",
                member.task,
                member.role.to_string(),
                member.agent.name(),
                member.status,
            ));
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
    let state = match round.life() {
        RoundLife::Open => "open".to_owned(),
        RoundLife::Closed { how, .. } => format!("closed, {how}"),
    };
    format!("{}  {item}  {state}", round.id())
}

/// A round in full as text: its line, the item, the pull request, the review passes, and the tasks that worked for it
/// with where they work and their runs.
pub(crate) fn render_round(detail: &RoundDetail) -> String {
    let mut lines = vec![format!("{}  team {}", round_line(&detail.round), detail.team)];
    lines.push(format!("  opened {}", detail.round.opened_at()));
    if let RoundLife::Closed { at, how } = detail.round.life() {
        lines.push(format!("  closed {at}, {how}"));
    }
    if let Some(item) = &detail.item {
        lines.push(format!("  item {}: {} ({})", item.id, item.title, item.status));
    }
    if let Some(pr) = &detail.pr {
        lines.push(format!("  pull request #{} on {}", pr.number, pr.branch));
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
                RunState::Ended { at, .. } => format!("ended {at}"),
            };
            lines.push(format!("    {}  started {}  {state}", run.id(), run.started_at()));
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// What is written when the daemon knows no team.
const NO_TEAMS: &str = "no teams\n";

/// What `agent status --running` writes when no round goes on.
const NO_OPEN_ROUNDS: &str = "no team has an open round\n";

#[cfg(test)]
#[path = "status/history/tests.rs"]
mod history_tests;
#[cfg(test)]
mod tests;
