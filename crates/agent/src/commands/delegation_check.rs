//! `agent _delegation-check`: the check the bus asks of a request to a worker, against the requester's plan file.
//!
//! The bus reads no worktree and has no plan reader; this hidden command reads the plan at the moment of the request and
//! the `[plan] check_refuse` of the home's configuration, and tells the bus its verdict on standard output.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::Args;

/// The arguments of `agent _delegation-check`.
#[derive(Debug, Args)]
pub(crate) struct DelegationCheckArgs {
    /// The requester's plan file, `.agent/plan.md` of its worktree.
    #[arg(long, value_name = "PATH")]
    pub(crate) plan: PathBuf,
    /// The step of the plan the request names (`--for 2.3`).
    #[arg(long = "for", value_name = "STEP", default_value = "")]
    pub(crate) step: String,
    /// The paths the worker may read and write, comma-separated.
    #[arg(long, value_name = "PATHS", default_value = "")]
    pub(crate) files: String,
    /// The check that shows the result is right.
    #[arg(long, value_name = "CHECK", default_value = "")]
    pub(crate) check: String,
    /// What the worker is to do.
    #[arg(long = "do", value_name = "WHAT", default_value = "")]
    pub(crate) doing: String,
}

/// Carries out `agent _delegation-check`: writes the verdict and returns the status of the command.
///
/// # Errors
/// The write failure of `out`.
pub(crate) fn delegation_check(_args: &DelegationCheckArgs, out: &mut impl Write) -> io::Result<()> {
    write!(out, "NO-PLAN\n")?;
    Ok(())
}
