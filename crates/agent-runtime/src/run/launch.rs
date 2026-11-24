//! Making a run ready to start: its request, its files, its process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::cli::{OutputParser, RunRequest};
use agent_core::config::{AbsolutePath, EnvName};
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables};

use super::{EngineParts, ExcludeCause, MAX_PROMPT_ARGUMENT_BYTES, RunError, RunJob};
use crate::cli::adapter_for;
use crate::commit_guard::hooks::install_hooks;
use crate::git::{CommitIdentity, Git};
use crate::process::{ProcessSpec, error_chain};
use crate::store::HOME_VARIABLE;
use tokio::io::AsyncWriteExt;

/// The file of a run's standard output, in its directory.
pub(super) const STDOUT_FILE: &str = "out.jsonl";

/// The file of a run's standard error, in its directory.
const STDERR_FILE: &str = "err.log";

/// The file a prompt too long for a command line is written to, in the run's directory.
const PROMPT_FILE: &str = "prompt.md";

/// The directory of a run, in its run directory, that holds the hooks its `git` is pointed at.
const HOOKS_DIR: &str = "hooks";

/// The program the hook runs: the executable of the harness, which sits in the directory of the agents' `PATH`.
const AGENT_PROGRAM: &str = "agent";

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
    /// Whether the prompt was written to a file instead of passed on the command line.
    pub(super) prompt_in_file: bool,
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

/// The names of the environment variables that carry the `n`th item of the configuration given to git: its key and its
/// value. Git reads as many as `GIT_CONFIG_COUNT` says.
const CONFIG_VARIABLES: [(&str, &str); 4] = [
    ("GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0"),
    ("GIT_CONFIG_KEY_1", "GIT_CONFIG_VALUE_1"),
    ("GIT_CONFIG_KEY_2", "GIT_CONFIG_VALUE_2"),
    ("GIT_CONFIG_KEY_3", "GIT_CONFIG_VALUE_3"),
];

/// The configuration of the `git` of a run, given in its environment so that it wins over the files of the human and of
/// the repository: how commits are signed, and where the hooks are (`hooks`, when the run has them).
fn git_config(identity: &CommitIdentity, hooks: Option<&Path>) -> Result<Vec<(&'static str, String)>, RunError> {
    let mut config = Vec::new();
    match identity.signing_key() {
        Some(key) => config.extend([
            ("commit.gpgsign", "true".to_owned()),
            ("gpg.format", "ssh".to_owned()),
            ("user.signingkey", utf8(key.as_path())?.to_owned()),
        ]),
        // The human's own configuration may sign every commit with a key the agent does not have.
        None => config.push(("commit.gpgsign", "false".to_owned())),
    }
    if let Some(hooks) = hooks {
        config.push(("core.hooksPath", utf8(hooks)?.to_owned()));
    }
    Ok(config)
}

/// What makes the `git` an agent runs commit as `identity`: the author and the committer, and whether and with which
/// key a commit is signed. Without it the commits of a run are the human's, whose configuration git reads. `hooks` is
/// the hooks directory of the run, if it has one.
fn identity_env(identity: &CommitIdentity, hooks: Option<&Path>) -> Result<Vec<(&'static str, String)>, RunError> {
    let mut env = vec![
        ("GIT_AUTHOR_NAME", identity.name().to_owned()),
        ("GIT_AUTHOR_EMAIL", identity.email().to_owned()),
        ("GIT_COMMITTER_NAME", identity.name().to_owned()),
        ("GIT_COMMITTER_EMAIL", identity.email().to_owned()),
    ];
    let config = git_config(identity, hooks)?;
    env.push(("GIT_CONFIG_COUNT", config.len().to_string()));
    for ((key, value), (key_variable, value_variable)) in config.into_iter().zip(CONFIG_VARIABLES) {
        env.push((key_variable, key.to_owned()));
        env.push((value_variable, value));
    }
    Ok(env)
}

/// Makes the hooks directory of a run in its `run_dir`, which `core.hooksPath` is to name. A checkout that cannot be
/// given hooks (it is no repository, or git fails) is run without them and the reason is logged: the checks before the
/// review look at the commits again.
async fn guard_hooks(parts: &EngineParts, job: &RunJob, run_dir: &Path) -> Option<PathBuf> {
    let agent = parts.agent_binary_dir.as_path().join(AGENT_PROGRAM);
    match install_hooks(&lookup_git(&job.working_dir), &run_dir.join(HOOKS_DIR), &agent).await {
        Ok(hooks) => Some(hooks),
        Err(error) => {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                task = %job.task,
                error = %error_chain(&error),
                "the git of the run gets no hook that refuses a commit of a frozen test"
            );
            None
        }
    }
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
    let (prompt, prompt_in_file) = argument_prompt(parts, job, &run_dir).await?;
    let request = RunRequest {
        program: parts.config.binaries().program(job.cli).clone(),
        model: job.model.clone(),
        extra_args: job.extra_args.clone(),
        mode: job.mode.clone(),
        prompt,
        working_dir: job.working_dir.clone(),
        run_dir: absolute(&run_dir)?,
        mcp_servers: job.mcp_servers.clone(),
        context: job.context,
        output: job.output,
    };
    let adapter = adapter_for(job.cli);
    let spec = adapter
        .build_run_with_max_turns(&request, parts.config.limits().max_turns)
        .map_err(|source| RunError::Build {
            task: job.task.clone(),
            source,
        })?;
    for file in &spec.files {
        write_file(file.path.as_path(), &file.content).await?;
    }
    keep_out_of_git(&spec.working_dir, &spec.worktree_excludes)
        .await
        .map_err(|source| RunError::Exclude {
            worktree: spec.working_dir.as_path().to_path_buf(),
            source,
        })?;
    let hooks = guard_hooks(parts, job, &run_dir).await;
    let mut env = base_agent_environment(
        &parts.secrets,
        &parts.agent_binary_dir,
        &parts.config.binaries().agent_path,
        parts.store.home().root(),
        &job.identity,
        hooks.as_deref(),
    )?;
    // What the adapter sets is what its CLI needs to work: it wins over a secret of the same name.
    env.extend(spec.env);
    // The configured cache policy wins over inherited and secret environment values.
    let incremental = if parts.config.build().incremental { "1" } else { "0" };
    env.insert(env_name("CARGO_INCREMENTAL")?, incremental.to_owned());
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
        prompt_in_file,
    })
}

/// The prompt as the CLI gets it: itself (`false`), or — when it is too long for a command line — the text that sends
/// the agent to a file that holds it (`true`).
async fn argument_prompt(parts: &EngineParts, job: &RunJob, run_dir: &Path) -> Result<(String, bool), RunError> {
    if job.prompt.len() <= MAX_PROMPT_ARGUMENT_BYTES {
        return Ok((job.prompt.clone(), false));
    }
    let file: PathBuf = run_dir.join(PROMPT_FILE);
    write_file(&file, &job.prompt).await?;
    let values = Variables::new()
        .with("path", utf8(&file)?)
        .map_err(|source| RunError::PromptFileVariable { source })?;
    let prompt = parts
        .texts
        .render(HarnessPrompt::PromptFile, &values)
        .map_err(|source| RunError::PromptFile { source })?;
    Ok((prompt, true))
}

/// The directories of the `PATH` the agents run with: the daemon executable directory first, then the configured
/// `agent_path` directories without duplicating the daemon directory.
#[must_use]
pub fn agent_path_directories<'a>(agent_binary_dir: &'a Path, agent_path: &'a [AbsolutePath]) -> Vec<&'a Path> {
    let mut directories = Vec::with_capacity(agent_path.len() + 1);
    directories.push(agent_binary_dir);
    for path in agent_path {
        if path.as_path() != agent_binary_dir {
            directories.push(path.as_path());
        }
    }
    directories
}

/// Git in `worktree`, to read from: no commit is made, but a handle has an identity.
pub(super) fn lookup_git(worktree: &AbsolutePath) -> Git {
    Git::new(
        worktree.clone(),
        CommitIdentity::new(LOOKUP_NAME.to_owned(), LOOKUP_EMAIL.to_owned(), None),
    )
}

/// Adds the `patterns` to the file that keeps paths out of git for the worktree (`info/exclude`, shared by every
/// worktree of its repository), those it does not have yet.
///
/// # Errors
/// [`ExcludeCause::Git`] when git cannot say where the file is (the directory is no repository);
/// [`ExcludeCause::Io`] when the file cannot be read or written.
pub async fn keep_out_of_git(worktree: &AbsolutePath, patterns: &[String]) -> Result<(), ExcludeCause> {
    if patterns.is_empty() {
        return Ok(());
    }
    let git = lookup_git(worktree);
    let file = git.info_exclude_file().await?;
    let io_error = |source| ExcludeCause::Io {
        path: file.clone(),
        source,
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

/// The base environment of a run or command check of an agent with `identity`: the agents' secrets, `PATH`,
/// `AGENTS_HOME`, and the commit identity (with optional `hooks`).
fn base_agent_environment(
    secrets: &agent_core::config::Secrets,
    agent_binary_dir: &AbsolutePath,
    agent_path: &[AbsolutePath],
    home: &Path,
    identity: &CommitIdentity,
    hooks: Option<&Path>,
) -> Result<BTreeMap<EnvName, String>, RunError> {
    let mut env = BTreeMap::new();
    for (name, secret) in secrets.agent_env() {
        env.insert(name.clone(), secret.expose().to_owned());
    }
    let directories = agent_path_directories(agent_binary_dir.as_path(), agent_path);
    let joined = std::env::join_paths(directories).map_err(|source| RunError::AgentPath { source })?;
    let path = joined
        .into_string()
        .map_err(|path| RunError::PathNotUtf8 { path: path.into() })?;
    env.insert(env_name(PATH_VARIABLE)?, path);
    // The `agent` commands the run starts (`agent gate`) find the daemon of this home.
    env.insert(env_name(HOME_VARIABLE)?, utf8(home)?.to_owned());
    // The commits of the run are the agent's: this wins over a secret of the same name.
    for (name, value) in identity_env(identity, hooks)? {
        env.insert(env_name(name)?, value);
    }
    Ok(env)
}

/// The environment of a command check of a task whose agent has `identity`: the variables a run of that agent gets from
/// the daemon (the agents' secrets, `PATH` and `HOME`, the commit identity), and no other secret of the daemon.
///
/// # Errors
/// [`RunError`] when the `PATH` cannot be joined or is not UTF-8, or `home` is not UTF-8.
pub fn check_environment(
    secrets: &agent_core::config::Secrets,
    agent_binary_dir: &AbsolutePath,
    agent_path: &[AbsolutePath],
    home: &Path,
    identity: &CommitIdentity,
) -> Result<Vec<(String, String)>, RunError> {
    let env = base_agent_environment(secrets, agent_binary_dir, agent_path, home, identity, None)?;
    Ok(env.into_iter().map(|(k, v)| (k.as_str().to_owned(), v)).collect())
}

#[cfg(test)]
mod tests;
