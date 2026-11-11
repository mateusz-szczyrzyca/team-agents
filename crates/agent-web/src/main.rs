//! The `agent-web` dashboard: a web view of the `agent` daemon (see `docs/architecture.md`).
//!
//! Its server arrives with a later item of the plan; this version answers `--version` and `--help`.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

/// The command line of `agent-web`.
#[derive(Debug, Parser)]
#[command(name = "agent-web", version, about = "Dashboard of the agent daemon")]
struct Cli {}

/// Why `agent-web` could not finish its output.
#[derive(Debug, thiserror::Error)]
enum OutputError {
    /// Standard output or standard error refused the text (a closed pipe, a full disk).
    #[error("cannot write the output of agent-web")]
    Write(#[source] io::Error),
}

/// Runs `agent-web` with the command line `args` (the program name first), writing to `out` and `err`.
///
/// # Errors
/// [`OutputError::Write`] when `out` or `err` refuses the text.
fn run<I, T>(args: I, out: &mut impl Write, err: &mut impl Write) -> Result<ExitCode, OutputError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let _ = (args.into_iter().count(), out, err);
    Ok(ExitCode::SUCCESS)
}

fn main() -> Result<ExitCode, OutputError> {
    run(std::env::args_os(), &mut io::stdout().lock(), &mut io::stderr().lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `agent-web` with `args` and returns its exit status, standard output and standard error.
    fn agent_web(args: &[&str]) -> (ExitCode, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            std::iter::once("agent-web").chain(args.iter().copied()),
            &mut out,
            &mut err,
        )
        .unwrap();
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
        let (code, out, err) = agent_web(&["--version"]);

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(out, format!("agent-web {}\n", env!("CARGO_PKG_VERSION")));
        assert_eq!(err, "");
    }

    #[test]
    fn help_goes_to_stdout_and_succeeds() {
        let (code, out, err) = agent_web(&["--help"]);

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(out.starts_with("Dashboard of the agent daemon\n"), "{out}");
        assert!(out.contains("Usage: agent-web"), "{out}");
        assert_eq!(err, "");
    }

    #[test]
    fn no_arguments_print_the_help_and_succeed() {
        let (code, out, err) = agent_web(&[]);

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(out.contains("Usage: agent-web"), "{out}");
        assert_eq!(err, "");
    }

    #[test]
    fn an_unknown_argument_is_refused_on_stderr_with_status_2() {
        let (code, out, err) = agent_web(&["--bogus"]);

        assert_eq!(code, ExitCode::from(2));
        assert_eq!(out, "");
        assert!(err.contains("unexpected argument '--bogus'"), "{err}");
    }

    #[test]
    fn a_closed_stdout_is_an_error_not_a_panic() {
        let result = run(["agent-web", "--version"], &mut ClosedPipe, &mut Vec::new());

        assert!(matches!(result, Err(OutputError::Write(e)) if e.kind() == io::ErrorKind::BrokenPipe));
    }

    #[test]
    fn a_closed_stderr_is_an_error_not_a_panic() {
        let result = run(["agent-web", "--bogus"], &mut Vec::new(), &mut ClosedPipe);

        assert!(matches!(result, Err(OutputError::Write(e)) if e.kind() == io::ErrorKind::BrokenPipe));
    }
}
