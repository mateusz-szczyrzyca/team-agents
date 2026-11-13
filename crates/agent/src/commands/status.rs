//! `agent status`: the teams the daemon knows, and what each of their tasks is doing.

use std::io::{self, Write};

use agent_core::protocol::{Client, Command, Reply, StatusRequest, Target, TeamSummary};

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
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target }))
        .await?;
    let Reply::Teams { teams } = reply else {
        return Err(StatusError::UnexpectedReply { kind: kind_of(&reply) });
    };
    out.write_all(render(&teams).as_bytes()).map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// The name of a kind of reply, for an error message.
pub(super) const fn kind_of(reply: &Reply) -> &'static str {
    match reply {
        Reply::Started { .. } => "started",
        Reply::Teams { .. } => "teams",
        Reply::Team { .. } => "team",
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
        for member in &team.members {
            let waiting = member.open_question.map_or_else(String::new, |round| {
                format!(", round {round} of questions waits for answers")
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

/// What is written when the daemon knows no team.
const NO_TEAMS: &str = "no teams\n";

#[cfg(test)]
mod tests;
