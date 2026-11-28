//! `agent swap`: swap the agent holding a task.

use std::io::Write;

use agent_core::ids::{AgentName, TaskId};
use agent_core::protocol::{Command, Failure, Reply, RequestId, ShowRequest, SwapRequest, Target};
use agent_core::status::TaskStatus;

use super::control::{CLIENT, ControlError, unexpected};
use crate::client::{ClientError, Connection};

#[cfg(test)]
mod tests;

/// Why a swap command could not complete.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SwapError {
    /// The socket failed or sent an unreadable response.
    #[error(transparent)]
    Client(ClientError),
    /// The daemon refused the requested handover.
    #[error("{}", .0.message)]
    Refused(Failure),
    /// An older decoder rejected the swap protocol before reading its request ID.
    #[error("the daemon does not know swap; restart it with this version")]
    OlderDaemon,
    /// The reply was unexpected or output could not be written.
    #[error(transparent)]
    Control(#[from] ControlError),
}

impl From<ClientError> for SwapError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Refused(failure) => Self::Refused(failure),
            ClientError::WrongAnswer {
                found: RequestId(0), ..
            } => Self::OlderDaemon,
            other => Self::Client(other),
        }
    }
}

/// Asks the daemon to swap `task` to `agent`.
///
/// # Errors
/// [`SwapError`].
pub(crate) async fn swap(
    connection: &mut Connection,
    task: TaskId,
    agent: AgentName,
    out: &mut impl Write,
) -> Result<(), SwapError> {
    let show = connection
        .request_swap(
            CLIENT,
            Command::Show(ShowRequest {
                target: Target::Task(task.clone()),
            }),
        )
        .await?;
    let Reply::Task { member, .. } = show else {
        return Err(unexpected(&show).into());
    };
    let old_agent = member.agent().name().to_string();
    let was_running = member.status() == TaskStatus::Running || member.current_run().is_some();

    let reply = connection
        .request_swap(
            CLIENT,
            Command::Swap(SwapRequest {
                task: task.clone(),
                agent: agent.clone(),
            }),
        )
        .await?;
    let Reply::Applied { .. } = reply else {
        return Err(unexpected(&reply).into());
    };

    let stopped = if was_running { " (run stopped)" } else { "" };
    writeln!(out, "{task}: {old_agent} → {agent}{stopped}").map_err(ControlError::Output)?;
    out.flush().map_err(ControlError::Output)?;
    Ok(())
}
