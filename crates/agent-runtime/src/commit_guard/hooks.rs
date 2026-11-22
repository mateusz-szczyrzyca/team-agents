//! The hooks directory an agent's `git` runs with: the repository's own hooks, and a `commit-msg` hook that asks the
//! harness whether the commit may be made (`agent guard-commit`).
//!
//! Git is pointed at the directory with `core.hooksPath` in the environment of the agent's run only, so the checkout of
//! the human and the files of the repository are not touched, and every CLI that commits through `git` is held to the
//! rule. The repository's own hooks keep running: the others are linked in, and its `commit-msg` runs after the guard
//! lets the commit through.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::git::{Git, GitError};

/// Why the hooks directory cannot be made.
#[derive(Debug, thiserror::Error)]
pub enum HooksError {
    /// Git cannot say where the repository keeps its hooks.
    #[error("cannot find the hooks of the repository")]
    Git(#[source] GitError),
    /// A path that goes into the hook is not plain text, or holds a line break or a NUL.
    #[error("the path {path:?} cannot be written into a hook")]
    Path {
        /// The path.
        path: PathBuf,
    },
    /// The directory, a link or the hook cannot be made.
    #[error("cannot make {path:?}")]
    Make {
        /// What was being made.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The repository's hooks cannot be read.
    #[error("cannot read the hooks in {path:?}")]
    Read {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
}

/// The hook that sees the message of a commit.
const COMMIT_MSG: &str = "commit-msg";

/// The ending of the example hooks git puts into a new repository: they are no hooks.
const SAMPLE_SUFFIX: &str = ".sample";

/// The mode of the hook: it is run by git.
const HOOK_MODE: u32 = 0o755;

/// `path` quoted for a shell, which `sh` reads as the one word it is.
fn quoted(path: &Path) -> Result<String, HooksError> {
    let text = path
        .to_str()
        .filter(|text| !text.contains(['\n', '\0']))
        .ok_or_else(|| HooksError::Path {
            path: path.to_path_buf(),
        })?;
    Ok(format!("'{}'", text.replace('\'', "'\\''")))
}

/// The text of the `commit-msg` hook that runs `agent` and then `original`, the repository's own hook, if it has one.
fn commit_msg_hook(agent: &Path, original: &Path) -> Result<String, HooksError> {
    let (agent, original) = (quoted(agent)?, quoted(original)?);
    Ok(format!(
        "#!/bin/sh\n\
         # Made by team-agents for the runs of an agent; it is not part of the repository.\n\
         {agent} guard-commit \"$1\" || exit \"$?\"\n\
         original={original}\n\
         if [ -x \"$original\" ]; then\n\
         \texec \"$original\" \"$@\"\n\
         fi\n"
    ))
}

/// Links every hook of `original` but `commit-msg` and the samples into `into`.
async fn link_hooks(original: &Path, into: &Path) -> Result<(), HooksError> {
    let read = |source| HooksError::Read {
        path: original.to_path_buf(),
        source,
    };
    let mut entries = match tokio::fs::read_dir(original).await {
        Ok(entries) => entries,
        // A repository without a hooks directory has no hooks to keep running.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(read(source)),
    };
    while let Some(entry) = entries.next_entry().await.map_err(read)? {
        let name = entry.file_name();
        if name == COMMIT_MSG || name.as_encoded_bytes().ends_with(SAMPLE_SUFFIX.as_bytes()) {
            continue;
        }
        let link = into.join(&name);
        match tokio::fs::remove_file(&link).await {
            Ok(()) => {}
            // A first installation has no link to replace.
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(HooksError::Make { path: link, source }),
        }
        tokio::fs::symlink(entry.path(), &link)
            .await
            .map_err(|source| HooksError::Make { path: link, source })?;
    }
    Ok(())
}

/// Makes the hooks directory `into` for the agent's runs in the checkout of `git`: links to the repository's hooks but
/// its `commit-msg`, and a `commit-msg` that runs `agent guard-commit` and then the repository's own. Returns `into`,
/// which is what `core.hooksPath` is to name.
///
/// # Errors
/// [`HooksError`].
pub async fn install_hooks(git: &Git, into: &Path, agent: &Path) -> Result<PathBuf, HooksError> {
    let original = git.git_path("hooks").await.map_err(HooksError::Git)?;
    let make = |path: &Path| {
        let path = path.to_path_buf();
        move |source| HooksError::Make { path, source }
    };
    tokio::fs::create_dir_all(into).await.map_err(make(into))?;
    link_hooks(&original, into).await?;
    let hook = into.join(COMMIT_MSG);
    let text = commit_msg_hook(agent, &original.join(COMMIT_MSG))?;
    // A hook of an earlier run is replaced whole, with its mode.
    tokio::fs::write(&hook, text).await.map_err(make(&hook))?;
    tokio::fs::set_permissions(&hook, std::fs::Permissions::from_mode(HOOK_MODE))
        .await
        .map_err(make(&hook))?;
    Ok(into.to_path_buf())
}
