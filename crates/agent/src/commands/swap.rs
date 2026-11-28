//! `agent swap`: swap the agent holding a task.

use std::io::Write;

use agent_core::ids::{AgentName, TaskId};
use agent_core::protocol::{Command, Reply, ShowRequest, SwapRequest, Target};
use agent_core::status::TaskStatus;

use super::control::{CLIENT, ControlError, unexpected};
use crate::client::Connection;

/// Asks the daemon to swap `task` to `agent`.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn swap(
    connection: &mut Connection,
    task: TaskId,
    agent: AgentName,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    let show = connection
        .request(
            CLIENT,
            Command::Show(ShowRequest {
                target: Target::Task(task.clone()),
            }),
        )
        .await?;
    let Reply::Task { member, .. } = show else {
        return Err(unexpected(&show));
    };
    let old_agent = member.agent().name().to_string();
    let was_running = member.status() == TaskStatus::Running;

    let reply = connection
        .request(
            CLIENT,
            Command::Swap(SwapRequest {
                task: task.clone(),
                agent: agent.clone(),
            }),
        )
        .await?;
    let Reply::Applied { .. } = reply else {
        return Err(unexpected(&reply));
    };

    let stopped = if was_running { " (run stopped)" } else { "" };
    writeln!(out, "{task}: {old_agent} → {agent}{stopped}").map_err(ControlError::Output)?;
    out.flush().map_err(ControlError::Output)
}
