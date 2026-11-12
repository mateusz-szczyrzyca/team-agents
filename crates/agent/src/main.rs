//! The `agent` command: starts and steers AI coding agents on this host (see `docs/architecture.md`).
//!
//! Its commands arrive with the later items of the plan; this version answers `--version` and `--help`.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use agent_core::ids::InvalidId;
use agent_core::protocol::Target;
use clap::{Args, CommandFactory, Parser, Subcommand};

mod client;
#[cfg(test)]
mod command_tests;
mod commands;
mod daemon;
mod environment;
#[cfg(test)]
mod fixtures;
mod lines;
#[cfg(test)]
mod testing;

use crate::client::{ClientError, Connection};
use crate::commands::status::{StatusError, status};
use crate::environment::{Environment, EnvironmentError};

/// Exit status of a command line that clap refuses: its own convention for usage errors.
const USAGE_ERROR: u8 = 2;

/// The command line of `agent`.
#[derive(Debug, Parser)]
#[command(name = "agent", version, about = "Starts and steers AI coding agents on this host")]
struct Cli {
    /// What to do; without one the help is shown.
    #[command(subcommand)]
    command: Option<Command>,
}

/// The commands of `agent`.
#[derive(Debug, Subcommand)]
enum Command {
    /// Shows the teams the daemon knows and what each of their tasks is doing.
    Status(StatusArgs),
}

/// The arguments of `agent status`.
#[derive(Debug, Args)]
struct StatusArgs {
    /// A team (`team-…`) or a task (`task-…`) to show; every team without one.
    #[arg(value_parser = parse_target)]
    target: Option<Target>,
}

/// Why a text is neither a team nor a task.
#[derive(Debug, thiserror::Error)]
#[error("{text:?} is neither a team ID (team-YYYYMMDD-HHMMSS-xxxx) nor a task ID (task-YYYYMMDD-HHMMSS-xxxx)")]
struct TargetError {
    /// What was given.
    text: String,
    /// What is wrong with it as an ID of the kind its prefix names.
    #[source]
    source: InvalidId,
}

/// Reads a team or a task ID from the command line.
fn parse_target(text: &str) -> Result<Target, TargetError> {
    let target = if text.starts_with("team-") {
        text.parse().map(Target::Team)
    } else {
        text.parse().map(Target::Task)
    };
    target.map_err(|source| TargetError {
        text: text.to_owned(),
        source,
    })
}

/// Why `agent` could not finish its output.
#[derive(Debug, thiserror::Error)]
enum OutputError {
    /// Standard output or standard error refused the text (a closed pipe, a full disk).
    #[error("cannot write the output of agent")]
    Write(#[source] io::Error),
}

/// Why a command failed.
#[derive(Debug, thiserror::Error)]
enum CommandError {
    /// `AGENTS_HOME` cannot be used.
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    /// The daemon cannot be reached or refused the request.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// `agent status` failed.
    #[error(transparent)]
    Status(#[from] StatusError),
    /// The tokio runtime cannot be built.
    #[error("cannot start the runtime of the command")]
    Runtime(#[source] io::Error),
}

/// Runs `agent` with the command line `args` (the program name first), writing to `out` and `err`.
///
/// # Errors
/// [`OutputError::Write`] when `out` or `err` refuses the text.
fn run<I, T>(args: I, out: &mut impl Write, err: &mut impl Write) -> Result<ExitCode, OutputError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_in(&Environment::from_process(), args, out, err)
}

/// Runs `agent` with the command line `args` in `environment`.
///
/// # Errors
/// [`OutputError::Write`] when `out` or `err` refuses the text.
fn run_in<I, T>(
    environment: &Environment,
    args: I,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<ExitCode, OutputError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let code = match Cli::try_parse_from(args) {
        // No command: the help says what there is.
        Ok(Cli { command: None }) => {
            write_text(out, &Cli::command().render_help().to_string())?;
            ExitCode::SUCCESS
        }
        Ok(Cli { command: Some(command) }) => match execute_blocking(environment, command, out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                write_text(err, &format!("agent: {}\n", chain(&error)))?;
                ExitCode::FAILURE
            }
        },
        // `--version` and `--help` arrive here too, as "errors" meant for stdout.
        Err(answer) if !answer.use_stderr() => {
            write_text(out, &answer.render().to_string())?;
            ExitCode::SUCCESS
        }
        Err(refusal) => {
            write_text(err, &refusal.render().to_string())?;
            ExitCode::from(USAGE_ERROR)
        }
    };
    Ok(code)
}

/// The error and its causes on one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Runs `command` on a runtime of its own.
fn execute_blocking(environment: &Environment, command: Command, out: &mut impl Write) -> Result<(), CommandError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(CommandError::Runtime)?;
    runtime.block_on(execute(environment, command, out))
}

/// Carries out `command`.
///
/// # Errors
/// [`CommandError`].
async fn execute(environment: &Environment, command: Command, out: &mut impl Write) -> Result<(), CommandError> {
    match command {
        Command::Status(args) => {
            let home = environment.home()?;
            let mut connection = Connection::connect(&home.socket_file()).await?;
            status(&mut connection, args.target, out).await?;
            Ok(())
        }
    }
}

/// Writes all of `text` to `to` and flushes it.
///
/// # Errors
/// [`OutputError::Write`] when `to` refuses the text.
fn write_text(to: &mut impl Write, text: &str) -> Result<(), OutputError> {
    to.write_all(text.as_bytes()).map_err(OutputError::Write)?;
    to.flush().map_err(OutputError::Write)
}

fn main() -> Result<ExitCode, OutputError> {
    run(std::env::args_os(), &mut io::stdout().lock(), &mut io::stderr().lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `agent` with `args` and returns its exit status, standard output and standard error.
    fn agent(args: &[&str]) -> (ExitCode, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(std::iter::once("agent").chain(args.iter().copied()), &mut out, &mut err).unwrap();
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    /// A writer that refuses every write, like a closed pipe.
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn version_prints_the_name_and_the_package_version_on_stdout() {
        let (code, out, err) = agent(&["--version"]);

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(out, format!("agent {}\n", env!("CARGO_PKG_VERSION")));
        assert_eq!(err, "");
    }

    #[test]
    fn help_goes_to_stdout_and_succeeds() {
        let (code, out, err) = agent(&["--help"]);

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            out.starts_with("Starts and steers AI coding agents on this host\n"),
            "{out}"
        );
        assert!(out.contains("Usage: agent"), "{out}");
        assert_eq!(err, "");
    }

    #[test]
    fn no_arguments_print_the_help_and_succeed() {
        let (code, out, err) = agent(&[]);

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(out.contains("Usage: agent"), "{out}");
        assert_eq!(err, "");
    }

    #[test]
    fn an_unknown_argument_is_refused_on_stderr_with_status_2() {
        let (code, out, err) = agent(&["--bogus"]);

        assert_eq!(code, ExitCode::from(2));
        assert_eq!(out, "");
        assert!(err.contains("unexpected argument '--bogus'"), "{err}");
    }

    #[test]
    fn a_closed_stdout_is_an_error_not_a_panic() {
        let result = run(["agent", "--version"], &mut ClosedPipe, &mut Vec::new());

        assert!(matches!(result, Err(OutputError::Write(e)) if e.kind() == io::ErrorKind::BrokenPipe));
    }

    #[test]
    fn a_closed_stderr_is_an_error_not_a_panic() {
        let result = run(["agent", "--bogus"], &mut Vec::new(), &mut ClosedPipe);

        assert!(matches!(result, Err(OutputError::Write(e)) if e.kind() == io::ErrorKind::BrokenPipe));
    }
}
