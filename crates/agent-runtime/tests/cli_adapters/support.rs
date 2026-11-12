//! What the tests of the four CLI adapters share: a run request, the recorded outputs and a replay of an output through
//! an adapter's parser. Nothing here starts a process.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;

use agent_core::cli::{ActivityEvent, CliAdapter, McpServerKind, OutputError, RunFile, RunMode, RunOutput, RunRequest};
use agent_core::config::{CliArgs, CommandLine};

/// A result whose error the test only reports.
pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The session ID the harness made up for a new run.
pub(crate) const MADE_UP_SESSION: &str = "5f1c2b9e-7d3a-4c0e-9a11-0b2c3d4e5f60";

/// The session ID of a conversation to go on with.
pub(crate) const EARLIER_SESSION: &str = "0a1b2c3d-1111-2222-3333-444455556666";

/// The MCP servers of a request.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Servers {
    /// None: a run that gets no MCP server.
    None,
    /// `agentbus serve` only.
    Bus,
    /// `agentbus serve` and Serena.
    BusAndSerena,
}

/// The command line the runtime gives for `agentbus serve`.
pub(crate) const BUS_WORDS: [&str; 6] = ["/opt/bin/agentbus", "serve", "--agent", "claude-1", "--team", "team-7"];

/// The command line the runtime gives for Serena, without the context the adapter adds.
pub(crate) const SERENA_WORDS: [&str; 6] = [
    "/opt/bin/serena",
    "start-mcp-server",
    "--project",
    "/work/tree",
    "--open-web-dashboard",
    "false",
];

/// A new conversation, with the ID the harness made up.
pub(crate) fn new_run() -> TestResult<RunMode> {
    Ok(RunMode::New {
        session_id: MADE_UP_SESSION.parse()?,
    })
}

/// The next turn of an earlier conversation.
pub(crate) fn resumed_run() -> TestResult<RunMode> {
    Ok(RunMode::Resume {
        session_id: EARLIER_SESSION.parse()?,
    })
}

/// A command line of `words`.
pub(crate) fn command(words: &[&str]) -> TestResult<CommandLine> {
    Ok(CommandLine::try_from(
        words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>(),
    )?)
}

/// A request for the program `/opt/bin/cli`, the model `model-1`, the extra arguments `--add-dir /extra` and a prompt
/// with a quote and a line break in it, run in `/work/tree` with `/work/run` for its own files.
pub(crate) fn request(mode: RunMode, servers: Servers) -> TestResult<RunRequest> {
    let mut mcp_servers = BTreeMap::new();
    if matches!(servers, Servers::Bus | Servers::BusAndSerena) {
        mcp_servers.insert(McpServerKind::Agentbus, command(&BUS_WORDS)?);
    }
    if matches!(servers, Servers::BusAndSerena) {
        mcp_servers.insert(McpServerKind::Serena, command(&SERENA_WORDS)?);
    }
    Ok(RunRequest {
        program: "/opt/bin/cli".parse()?,
        model: Some("model-1".parse()?),
        extra_args: CliArgs::try_from(vec!["--add-dir".to_owned(), "/extra".to_owned()])?,
        mode,
        prompt: "Do the item.\nQuote: \"it's\"".to_owned(),
        working_dir: "/work/tree".parse()?,
        run_dir: "/work/run".parse()?,
        mcp_servers,
    })
}

/// The text of the recorded output `name` of the CLI `cli`.
pub(crate) fn fixture(cli: &str, name: &str) -> TestResult<String> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", "cli", cli, name]
        .iter()
        .collect();
    Ok(std::fs::read_to_string(path)?)
}

/// The JSON a file of a run holds.
pub(crate) fn json_of(file: &RunFile) -> TestResult<serde_json::Value> {
    Ok(serde_json::from_str(&file.content)?)
}

/// What a parser made of an output.
#[derive(Debug)]
pub(crate) struct Replay {
    /// The steps the lines showed, in order.
    pub(crate) events: Vec<ActivityEvent>,
    /// The lines the parser could not read, in order.
    pub(crate) errors: Vec<OutputError>,
    /// What the run ended with.
    pub(crate) output: RunOutput,
}

/// Feeds `stdout` line by line into a new parser of `adapter`, then ends the run with `stderr` and a tail of 30 lines.
pub(crate) fn replay(adapter: &dyn CliAdapter, stdout: &str, stderr: &str) -> TestResult<Replay> {
    let mut parser = adapter.parser();
    let mut events = Vec::new();
    let mut errors = Vec::new();
    for line in stdout.lines() {
        match parser.push_line(line) {
            Ok(steps) => events.extend(steps),
            Err(error) => errors.push(error),
        }
    }
    let output = parser.finish(stderr, NonZeroU32::new(30).ok_or("30 is not zero")?);
    Ok(Replay { events, errors, output })
}
