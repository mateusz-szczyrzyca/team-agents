//! The hooks directory an agent's `git` runs with: the repository's own hooks, and a `commit-msg` hook that asks the
//! harness whether the commit may be made (`agent guard-commit`).
//!
//! Git is pointed at the directory with `core.hooksPath` in the environment of the agent's run only, so the checkout of
//! the human and the files of the repository are not touched, and every CLI that commits through `git` is held to the
//! rule. The repository's own hooks keep running: each hook of the directory runs the repository's hook of its name, and
//! the `commit-msg` runs after the guard lets the commit through.

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
    /// The directory or a hook cannot be made.
    #[error("cannot make {path:?}")]
    Make {
        /// What was being made.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
}

/// The hook that sees the message of a commit.
const COMMIT_MSG: &str = "commit-msg";

/// The exit status of `agent guard-commit` that refuses a commit.
const REFUSED: u8 = super::COMMIT_REFUSED_STATUS;

/// The mode of the hook: it is run by git.
const HOOK_MODE: u32 = 0o755;

/// The hooks git runs in a checkout: the hooks directory of a run has one for each, so that a commit in another
/// repository, made while the run's `core.hooksPath` is in the environment, finds that repository's own hooks.
const CLIENT_HOOKS: [&str; 17] = [
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "post-rewrite",
    "pre-auto-gc",
    "reference-transaction",
    "push-to-checkout",
    "post-index-change",
];

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

/// How a hook asks git what the repository itself says: the configuration of the environment (`GIT_CONFIG_COUNT`, and
/// `GIT_CONFIG_PARAMETERS`, which git fills from `git -c`) is taken away, so that the hooks path of the run is not the
/// answer.
const PLAIN_GIT: &str = "GIT_CONFIG_COUNT=0 GIT_CONFIG_PARAMETERS=";

/// What a hook says about itself, and how it finds the hook that is to run: in the repository it was made for, the hook
/// `original`; in another, that repository's own hook of this `name`. A hook never runs itself.
fn chain(name: &str, common_dir: &str, original: &str) -> String {
    format!(
        "# Made by team-agents for the runs of an agent; it is not part of the repository.\n\
         mine={common_dir}\n\
         here=$({PLAIN_GIT} git rev-parse --path-format=absolute --git-common-dir)\n\
         if [ \"$here\" = \"$mine\" ]; then\n\
         \ttarget={original}\n\
         else\n\
         \ttarget=$({PLAIN_GIT} git rev-parse --git-path hooks/{name})\n\
         fi\n\
         if [ -x \"$target\" ] && [ ! \"$target\" -ef \"$0\" ]; then\n\
         \texec \"$target\" \"$@\"\n\
         fi\n"
    )
}

/// The text of the hook `name`: it runs the repository's own hook of that name, and `commit-msg` asks `agent` first, for
/// the repository it was made for only.
fn hook_text(name: &str, agent: &Path, common_dir: &Path, original: &Path) -> Result<String, HooksError> {
    let (common_dir, original) = (quoted(common_dir)?, quoted(original)?);
    let guard = if name == COMMIT_MSG {
        let agent = quoted(agent)?;
        // Only the refusal of the guard (its status) stops the commit: a program that is missing or fails in another way
        // is a fault of the harness, and the checks before the review look at the commit again.
        format!(
            "agent={agent}\n\
             here=$({PLAIN_GIT} git rev-parse --path-format=absolute --git-common-dir)\n\
             if [ \"$here\" = {common_dir} ] && [ -x \"$agent\" ]; then\n\
             \t\"$agent\" guard-commit \"$1\"\n\
             \tif [ \"$?\" -eq {REFUSED} ]; then exit {REFUSED}; fi\n\
             fi\n"
        )
    } else {
        String::new()
    };
    Ok(format!(
        "#!/bin/sh\n{guard}{chain}",
        chain = chain(name, &common_dir, &original)
    ))
}

/// Makes the hooks directory `into` for the agent's runs in the checkout of `git`: a hook for each hook git runs, which
/// runs the repository's own hook of that name — the repository's `core.hooksPath` followed, a hooks path of the
/// environment ignored — and, for `commit-msg`, asks `agent guard-commit` first. The hooks act for this repository only:
/// in another one they run that repository's own hooks. Returns `into`, which is what `core.hooksPath` is to name.
///
/// # Errors
/// [`HooksError`].
pub async fn install_hooks(git: &Git, into: &Path, agent: &Path) -> Result<PathBuf, HooksError> {
    let own = git.clone().without_ambient_config();
    let original = own.git_path("hooks").await.map_err(HooksError::Git)?;
    let common_dir = own.common_dir().await.map_err(HooksError::Git)?;
    let make = |path: &Path| {
        let path = path.to_path_buf();
        move |source| HooksError::Make { path, source }
    };
    tokio::fs::create_dir_all(into).await.map_err(make(into))?;
    for name in CLIENT_HOOKS {
        let hook = into.join(name);
        let text = hook_text(name, agent, &common_dir, &original.join(name))?;
        // A hook of an earlier run is replaced whole, with its mode.
        tokio::fs::write(&hook, text).await.map_err(make(&hook))?;
        tokio::fs::set_permissions(&hook, std::fs::Permissions::from_mode(HOOK_MODE))
            .await
            .map_err(make(&hook))?;
    }
    Ok(into.to_path_buf())
}
