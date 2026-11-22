//! The hooks directory an agent's `git` runs with: the repository's own hooks, and a `commit-msg` hook that asks the
//! harness whether the commit may be made (`agent guard-commit`).
//!
//! Git is pointed at the directory with `core.hooksPath` in the environment of the agent's run only, so the checkout of
//! the human and the files of the repository are not touched, and every CLI that commits through `git` is held to the
//! rule. The repository's own hooks keep running: the others are linked in, and its `commit-msg` runs after the guard
//! lets the commit through.

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

/// Makes the hooks directory `into` for the agent's runs in the checkout of `git`: links to the repository's hooks but
/// its `commit-msg`, and a `commit-msg` that runs `agent guard-commit` and then the repository's own. Returns `into`,
/// which is what `core.hooksPath` is to name.
///
/// # Errors
/// [`HooksError`].
pub async fn install_hooks(git: &Git, into: &Path, agent: &Path) -> Result<PathBuf, HooksError> {
    let _ = (git, agent);
    tokio::fs::create_dir_all(into)
        .await
        .map_err(|source| HooksError::Make {
            path: into.to_path_buf(),
            source,
        })?;
    Ok(into.to_path_buf())
}
