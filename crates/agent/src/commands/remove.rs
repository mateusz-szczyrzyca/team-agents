//! `agent remove <team>`: asks the daemon to take a team out, and prints what was removed, bundled, closed, and what
//! stayed with its reason.

use std::io::Write;

use agent_core::ids::TeamId;
use agent_core::protocol::{Command, RemoveRequest, Reply};

use super::control::{CLIENT, ControlError, unexpected};
use crate::client::Connection;

/// Sends the removal of `team` and prints its outcome: the bundle, the branches, the pull requests closed, the bytes freed,
/// the dirty worktrees removed with their changes, and what was left behind.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn remove(
    connection: &mut Connection,
    team: TeamId,
    force: bool,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    let reply = connection
        .request(CLIENT, Command::Remove(RemoveRequest { team, force }))
        .await?;
    let Reply::Removed { removal } = reply else {
        return Err(unexpected(&reply));
    };
    let mut write = |text: String| writeln!(out, "{text}").map_err(ControlError::Output);
    write(format!(
        "Removed team {}: {} task(s).",
        removal.team,
        removal.tasks.len()
    ))?;
    for task in &removal.stopped {
        write(format!("  stopped the run of {task} first"))?;
    }
    if let Some(bundle) = &removal.bundle {
        write(format!(
            "  unmerged branches bundled in {}:",
            bundle.as_path().display()
        ))?;
        for branch in &removal.bundled {
            write(format!("    {branch}"))?;
        }
    }
    for branch in &removal.branches {
        write(format!("  deleted local branch {branch}"))?;
    }
    for number in &removal.closed_pull_requests {
        write(format!("  closed pull request #{number}"))?;
    }
    for path in &removal.dirty_worktrees {
        write(format!(
            "  worktree with uncommitted changes removed: {}",
            path.as_path().display()
        ))?;
    }
    write(format!("  freed {} bytes", removal.freed_bytes))?;
    for left in &removal.left_behind {
        write(format!("  left behind: {left}"))?;
    }
    out.flush().map_err(ControlError::Output)
}
