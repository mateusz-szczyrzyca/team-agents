//! Making a run ready to start: its request, its files, its process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::cli::{OutputParser, RunRequest};
use agent_core::config::{AbsolutePath, EnvName};
use agent_core::prompt::{HarnessPrompt, Variables};

use super::{EngineParts, MAX_PROMPT_ARGUMENT_BYTES, RunError, RunJob};
use crate::cli::adapter_for;
use crate::git::{CommitIdentity, Git};
use crate::process::ProcessSpec;
use crate::store::HOME_VARIABLE;
use tokio::io::AsyncWriteExt;

/// The file of a run's standard output, in its directory.
const STDOUT_FILE: &str = "out.jsonl";

/// The file of a run's standard error, in its directory.
const STDERR_FILE: &str = "err.log";

/// The file a prompt too long for a command line is written to, in the run's directory.
const PROMPT_FILE: &str = "prompt.md";

/// The environment variable the agents' programs are looked up through.
const PATH_VARIABLE: &str = "PATH";

/// The identity git is run with to find a file: no commit is made, but a handle has one.
const LOOKUP_NAME: &str = "team-agents";

/// The e-mail of that identity.
const LOOKUP_EMAIL: &str = "team-agents@team-agents.invalid";

/// A run that is ready to start.
pub(super) struct Launch {
    /// The process to start.
    pub(super) process: ProcessSpec,
    /// What reads its output.
    pub(super) parser: Box<dyn OutputParser>,
}

/// `path` as text, which the adapters need.
fn utf8(path: &Path) -> Result<&str, RunError> {
    path.to_str().ok_or_else(|| RunError::PathNotUtf8 {
        path: path.to_path_buf(),
    })
}

/// `path` as an absolute path.
fn absolute(path: &Path) -> Result<AbsolutePath, RunError> {
    AbsolutePath::from_str(utf8(path)?).map_err(|source| RunError::Path {
        path: path.to_path_buf(),
        source,
    })
}

/// A variable name that is a literal of this module.
fn env_name(name: &'static str) -> Result<EnvName, RunError> {
    EnvName::from_str(name).map_err(|source| RunError::EnvName { name, source })
}

/// Writes `content` to `path`, with its directories.
async fn write_file(path: &Path, content: &str) -> Result<(), RunError> {
    let lay_down = |source| RunError::LayDown {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(lay_down)?;
    }
    tokio::fs::write(path, content).await.map_err(lay_down)
}

/// Makes `job` ready: the prompt, the adapter's run, its files, what it keeps out of git, its process.
pub(super) async fn prepare(parts: &EngineParts, job: &RunJob) -> Result<Launch, RunError> {
    let run_dir = parts.store.home().run_dir(&job.task, &job.run);
    let prompt = argument_prompt(parts, job, &run_dir).await?;
    let request = RunRequest {
        program: parts.config.binaries().program(job.cli).clone(),
        model: job.model.clone(),
        extra_args: job.extra_args.clone(),
        mode: job.mode.clone(),
        prompt,
        working_dir: job.working_dir.clone(),
        run_dir: absolute(&run_dir)?,
        mcp_servers: job.mcp_servers.clone(),
    };
    let adapter = adapter_for(job.cli);
    let spec = adapter.build_run(&request).map_err(|source| RunError::Build {
        task: job.task.clone(),
        source,
    })?;
    for file in &spec.files {
        write_file(file.path.as_path(), &file.content).await?;
    }
    keep_out_of_git(&spec.working_dir, &spec.worktree_excludes).await?;
    let mut env = BTreeMap::new();
    for (name, secret) in parts.secrets.agent_env() {
        env.insert(name.clone(), secret.expose().to_owned());
    }
    env.insert(env_name(PATH_VARIABLE)?, agent_path(parts)?);
    // The `agent` commands the run starts (`agent gate`) find the daemon of this home.
    env.insert(env_name(HOME_VARIABLE)?, utf8(parts.store.home().root())?.to_owned());
    // What the adapter sets is what its CLI needs to work: it wins over a secret of the same name.
    env.extend(spec.env);
    Ok(Launch {
        process: ProcessSpec {
            program: spec.program,
            args: spec.args,
            env,
            working_dir: spec.working_dir,
            stdout_file: run_dir.join(STDOUT_FILE),
            stderr_file: run_dir.join(STDERR_FILE),
        },
        parser: adapter.parser(),
    })
}

/// The prompt as the CLI gets it: itself, or — when it is too long for a command line — the text that sends the agent
/// to a file that holds it.
async fn argument_prompt(parts: &EngineParts, job: &RunJob, run_dir: &Path) -> Result<String, RunError> {
    if job.prompt.len() <= MAX_PROMPT_ARGUMENT_BYTES {
        return Ok(job.prompt.clone());
    }
    let file: PathBuf = run_dir.join(PROMPT_FILE);
    write_file(&file, &job.prompt).await?;
    let values = Variables::new()
        .with("path", utf8(&file)?)
        .map_err(|source| RunError::PromptFileVariable { source })?;
    parts
        .texts
        .render(HarnessPrompt::PromptFile, &values)
        .map_err(|source| RunError::PromptFile { source })
}

/// The `PATH` of the agents: the directories of `[binaries] agent_path`, in order.
fn agent_path(parts: &EngineParts) -> Result<String, RunError> {
    let directories = parts.config.binaries().agent_path.iter().map(AbsolutePath::as_path);
    let joined = std::env::join_paths(directories).map_err(|source| RunError::AgentPath { source })?;
    joined
        .into_string()
        .map_err(|path| RunError::PathNotUtf8 { path: path.into() })
}

/// Git in `worktree`, to read from: no commit is made, but a handle has an identity.
pub(super) fn lookup_git(worktree: &AbsolutePath) -> Git {
    Git::new(
        worktree.clone(),
        CommitIdentity::new(LOOKUP_NAME.to_owned(), LOOKUP_EMAIL.to_owned(), None),
    )
}

/// Adds the `patterns` to the file that keeps paths out of git for the worktree, those it does not have yet.
async fn keep_out_of_git(worktree: &AbsolutePath, patterns: &[String]) -> Result<(), RunError> {
    if patterns.is_empty() {
        return Ok(());
    }
    let exclude_error = |source| RunError::Exclude {
        worktree: worktree.as_path().to_path_buf(),
        source,
    };
    let git = lookup_git(worktree);
    let file = git
        .info_exclude_file()
        .await
        .map_err(|source| exclude_error(super::ExcludeCause::Git(source)))?;
    let io_error = |source| {
        exclude_error(super::ExcludeCause::Io {
            path: file.clone(),
            source,
        })
    };
    let present = match tokio::fs::read_to_string(&file).await {
        Ok(text) => text,
        // A repository without the file has no exclusions yet: the file is made below.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(io_error(source)),
    };
    let missing: Vec<&String> = patterns
        .iter()
        .filter(|pattern| !present.lines().any(|line| line == pattern.as_str()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_error)?;
    }
    let mut added = String::new();
    if !present.is_empty() && !present.ends_with('\n') {
        added.push('\n');
    }
    for pattern in missing {
        added.push_str(pattern);
        added.push('\n');
    }
    // Appended, not rewritten: another run of the same repository may be adding its own lines.
    let mut handle = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .await
        .map_err(io_error)?;
    handle.write_all(added.as_bytes()).await.map_err(io_error)?;
    handle.flush().await.map_err(io_error)
}
