//! The `agent` command: starts and steers AI coding agents on this host (see `docs/architecture.md`).
//!
//! `agent daemon` runs the daemon; every other command is a client of its socket (`agent status`). The other commands
//! arrive with the later items of the plan.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use agent_core::ids::{AgentName, InvalidId, TaskId};
use agent_core::protocol::{SayError, Target};
use agent_core::state::MergeMode;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

mod client;
// Test code lives under `tests/`: the builders of states, the stand-in for the daemon's socket and the tests of the
// command line. The modules keep their names, so `crate::fixtures` is still `crate::fixtures`.
#[cfg(test)]
#[path = "tests/commands.rs"]
mod command_tests;
mod commands;
mod daemon;
mod environment;
#[cfg(test)]
#[path = "tests/fixtures.rs"]
mod fixtures;
mod lines;
#[cfg(test)]
#[path = "tests/run_command.rs"]
mod run_command_tests;
#[cfg(test)]
#[path = "tests/steer_commands.rs"]
mod steer_command_tests;
#[cfg(test)]
#[path = "tests/testing.rs"]
mod testing;

use crate::client::{ClientError, Connection};
use crate::commands::answer::{AnswerError, Source, answer};
use crate::commands::control::ControlError;
use crate::commands::daemon::{DaemonCommandError, daemon};
use crate::commands::pause::pause;
use crate::commands::resume::resume;
use crate::commands::run::{RunError, RunInput, prepare, start};
use crate::commands::say::{say, text as say_text};
use crate::commands::status::{StatusError, status};
use crate::commands::stop::stop;
use crate::commands::unpause::unpause;
use crate::environment::{EditorError, Environment, EnvironmentError};
use agent_runtime::process::error_chain;

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
    /// Runs the daemon: the owner of the state and of every model run, which the other commands talk to.
    Daemon(DaemonArgs),
    /// Shows the teams the daemon knows and what each of their tasks is doing.
    Status(StatusArgs),
}

/// The name of the command that starts a task or a team.
const RUN: &str = "run";

/// What `agent run` is for, as the help says.
const RUN_ABOUT: &str = "Starts a task or a team on the repository the current directory is in";

/// What a command line says: one of the commands of [`Cli`], or `run`.
///
/// `run` is added to the command line when it is parsed ([`parse`]) and is not a variant of [`Command`]: the tests of the
/// command line match that enum exhaustively, so a new command is kept out of it until they are changed.
#[derive(Debug)]
enum Parsed {
    /// `daemon`, `status` or no command.
    Cli(Cli),
    /// `run`.
    Run(RunArgs),
}

/// Parses `args` (the program name first) as `agent` takes them.
///
/// # Errors
/// [`clap::Error`] for a command line that is wrong, and for `--help` and `--version`, which clap shows as errors.
fn parse<I, T>(args: I) -> Result<Parsed, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut matches = command().try_get_matches_from(args)?;
    if matches.subcommand_name() == Some(RUN)
        && let Some((_, run)) = matches.remove_subcommand()
    {
        return Ok(Parsed::Run(RunArgs::from_arg_matches(&run)?));
    }
    Ok(Parsed::Cli(Cli::from_arg_matches(&matches)?))
}

/// The command line of `agent`: the commands of [`Cli`] and `run`; the parser and the help are made from it.
fn command() -> clap::Command {
    // The description is set after the arguments: `augment_args` gives the command the doc comment of `RunArgs`.
    let with_run = Cli::command().subcommand(RunArgs::augment_args(clap::Command::new(RUN)).about(RUN_ABOUT));
    // `augment_subcommands` gives the command the doc comment of `Steer` as its description: the description is kept.
    let about = with_run.get_about().cloned();
    let steering = Steer::augment_subcommands(with_run);
    match about {
        Some(about) => steering.about(about),
        None => steering,
    }
}

/// What a command line says, with the commands that steer: [`Parsed`] and [`Steer`].
///
/// [`Steer`] is kept out of [`Parsed`] and [`Command`] for the reason `run` is: the tests of the command line match
/// those enums exhaustively.
#[derive(Debug)]
enum Invocation {
    /// `daemon`, `status`, `run` or no command.
    Parsed(Parsed),
    /// `answer`, `stop`, `pause`, `unpause`, `resume` or `say`.
    Steer(Steer),
}

/// Parses `args` (the program name first) as `agent` takes them, the commands that steer included.
///
/// # Errors
/// [`clap::Error`] as for [`parse`].
fn parse_invocation<I, T>(args: I) -> Result<Invocation, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let matches = command().try_get_matches_from(&args)?;
    if matches.subcommand_name().is_some_and(Steer::has_subcommand) {
        return Ok(Invocation::Steer(Steer::from_arg_matches(&matches)?));
    }
    parse(args).map(Invocation::Parsed)
}

/// The arguments of `agent run`.
#[derive(Debug, Args)]
struct RunArgs {
    /// The agents to run, then the task unless it is in a file: one agent is a task of its own, more are a team.
    #[arg(required = true, value_name = "AGENTS... TASK")]
    words: Vec<String>,
    /// The file with the task list, instead of the last argument.
    #[arg(short = 'f', long = "file", value_name = "FILE")]
    file: Option<PathBuf>,
    /// The agent that coordinates the team; it wins over the `Coordinator:` line of the task list.
    #[arg(long, value_name = "NAME")]
    coordinator: Option<AgentName>,
    /// Stop before the merge of an approved pull request and hand it to you.
    #[arg(long)]
    no_merge: bool,
}

/// The commands that steer a task or a team, and answer the questions of a task.
#[derive(Debug, Subcommand)]
enum Steer {
    /// Answers the questions of a task, in your editor or from a file; the task goes on with the answers.
    Answer(AnswerArgs),
    /// Stops a task or a team: its run ends and nothing of it starts until it is resumed.
    Stop(TargetArgs),
    /// Pauses a task or a team: its run ends and nothing of it starts until it is unpaused.
    Pause(TargetArgs),
    /// Lets a paused task or team go on, in the session it was in.
    Unpause(TargetArgs),
    /// Starts a stopped, failed or waiting task or team again.
    Resume(TargetArgs),
    /// Says something to a task or a team, through the bus: they read it when they next look at their messages.
    Say(SayArgs),
}

/// The task or team a command is about.
#[derive(Debug, Args)]
struct TargetArgs {
    /// A team (`team-…`) or a task (`task-…`).
    #[arg(value_parser = parse_target)]
    target: Target,
}

/// The arguments of `agent answer`.
#[derive(Debug, Args)]
struct AnswerArgs {
    /// The task whose questions are answered (`task-…`).
    #[arg(value_parser = parse_task)]
    task: TaskId,
    /// A questions file with your answers after `**Verdict:**`, instead of your editor.
    #[arg(short = 'f', long = "file", value_name = "FILE")]
    file: Option<PathBuf>,
}

/// The arguments of `agent say`.
#[derive(Debug, Args)]
struct SayArgs {
    /// The team (`team-…`) or task (`task-…`) that is told.
    #[arg(value_parser = parse_target)]
    target: Target,
    /// What is said: the words are joined with spaces.
    #[arg(required = true, value_name = "WORDS")]
    words: Vec<String>,
}

/// The arguments of `agent daemon`.
#[derive(Debug, Args)]
struct DaemonArgs {
    /// The directory of the prompt files; `prompts/` in `AGENTS_HOME` when not given.
    #[arg(long, value_name = "DIR")]
    prompts_dir: Option<PathBuf>,
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

/// Why a text is not a task.
#[derive(Debug, thiserror::Error)]
#[error("{text:?} is not a task ID (task-YYYYMMDD-HHMMSS-xxxx)")]
struct TaskArgError {
    /// What was given.
    text: String,
    /// What is wrong with it.
    #[source]
    source: InvalidId,
}

/// Reads a task ID from the command line.
fn parse_task(text: &str) -> Result<TaskId, TaskArgError> {
    text.parse().map_err(|source| TaskArgError {
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
    /// `agent run` failed.
    #[error(transparent)]
    Run(#[from] RunError),
    /// `agent daemon` failed.
    #[error(transparent)]
    Daemon(#[from] DaemonCommandError),
    /// `agent stop`, `pause`, `unpause` or `resume` failed.
    #[error(transparent)]
    Control(#[from] ControlError),
    /// The words of `agent say` are not something the daemon takes.
    #[error("the words to say are not valid")]
    Words(#[from] SayError),
    /// `agent answer` failed.
    #[error(transparent)]
    Answer(#[from] AnswerError),
    /// The editor for `agent answer` cannot be found.
    #[error(transparent)]
    Editor(#[from] EditorError),
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
    let code = match parse_invocation(args) {
        // No command: the help says what there is.
        Ok(Invocation::Parsed(Parsed::Cli(Cli { command: None }))) => {
            write_text(out, &command().render_help().to_string())?;
            ExitCode::SUCCESS
        }
        Ok(Invocation::Parsed(Parsed::Cli(Cli { command: Some(command) }))) => {
            finish(execute_blocking(environment, Work::Command(command), out), err)?
        }
        Ok(Invocation::Parsed(Parsed::Run(args))) => finish(execute_blocking(environment, Work::Run(args), out), err)?,
        Ok(Invocation::Steer(steer)) => finish(execute_blocking(environment, Work::Steer(steer), out), err)?,
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

/// The exit status of a command that ended with `result`; a failure is written to `err` with its causes.
///
/// # Errors
/// [`OutputError::Write`] when `err` refuses the text.
fn finish(result: Result<(), CommandError>, err: &mut impl Write) -> Result<ExitCode, OutputError> {
    match result {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) => {
            write_text(err, &format!("agent: {}\n", error_chain(&error)))?;
            Ok(ExitCode::FAILURE)
        }
    }
}

/// What a command line asks to be done.
#[derive(Debug)]
enum Work {
    /// A command of [`Cli`].
    Command(Command),
    /// `run`.
    Run(RunArgs),
    /// A command that steers.
    Steer(Steer),
}

/// Runs `work` on a runtime of its own.
fn execute_blocking(environment: &Environment, work: Work, out: &mut impl Write) -> Result<(), CommandError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(CommandError::Runtime)?;
    runtime.block_on(async {
        match work {
            Work::Command(command) => execute(environment, command, out).await,
            Work::Run(args) => execute_run(environment, args, out).await,
            Work::Steer(steer) => execute_steer(environment, steer, out).await,
        }
    })
}

/// Carries out `command`.
///
/// # Errors
/// [`CommandError`].
async fn execute(environment: &Environment, command: Command, out: &mut impl Write) -> Result<(), CommandError> {
    match command {
        Command::Daemon(args) => Ok(daemon(environment, args.prompts_dir).await?),
        Command::Status(args) => {
            let home = environment.home()?;
            let mut connection = Connection::connect(&home.socket_file()).await?;
            status(&mut connection, args.target, out).await?;
            Ok(())
        }
    }
}

/// Carries out a command that steers, as the command line.
///
/// # Errors
/// [`CommandError`].
async fn execute_steer(environment: &Environment, steer: Steer, out: &mut impl Write) -> Result<(), CommandError> {
    let home = environment.home()?;
    let socket = home.socket_file();
    match steer {
        Steer::Answer(AnswerArgs { task, file }) => {
            // The editor is found before the daemon is asked: with none there is nothing to answer with.
            let source = match file {
                Some(file) => Source::File(file),
                None => Source::Editor(environment.editor(&home).await?),
            };
            answer(&mut Connection::connect(&socket).await?, task, source, out).await?;
        }
        Steer::Stop(args) => stop(&mut Connection::connect(&socket).await?, args.target, out).await?,
        Steer::Pause(args) => pause(&mut Connection::connect(&socket).await?, args.target, out).await?,
        Steer::Unpause(args) => unpause(&mut Connection::connect(&socket).await?, args.target, out).await?,
        Steer::Resume(args) => resume(&mut Connection::connect(&socket).await?, args.target, out).await?,
        Steer::Say(args) => {
            // The words are checked before the daemon is asked.
            let text = say_text(&args.words)?;
            say(&mut Connection::connect(&socket).await?, args.target, text, out).await?;
        }
    }
    Ok(())
}

/// Carries out `agent run`.
///
/// # Errors
/// [`CommandError`].
async fn execute_run(environment: &Environment, args: RunArgs, out: &mut impl Write) -> Result<(), CommandError> {
    let directory = environment.directory()?;
    let home = environment.home()?;
    let merge = if args.no_merge {
        MergeMode::NoMerge
    } else {
        MergeMode::Autopilot
    };
    // Everything the command can find wrong by itself is found before the daemon is looked for.
    let request = prepare(
        directory,
        RunInput {
            words: args.words,
            file: args.file,
            coordinator: args.coordinator,
            merge,
        },
    )
    .await?;
    let mut connection = Connection::connect(&home.socket_file()).await?;
    start(&mut connection, &home, request, out).await?;
    Ok(())
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
