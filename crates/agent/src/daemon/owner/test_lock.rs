//! Whether the test lock of the machine (`tools/test-lock.sh`) is held by a test run of one worktree.
//!
//! The lock is an `flock` on one file. Its holder writes who it is into the file (`dir=` is the directory the run was
//! started in); the file stays when the lock is let go, so what it says counts only while the lock is held.

use std::fs::{File, TryLockError};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The file of the test lock when `TEST_LOCK_FILE` does not name another.
pub(super) const DEFAULT_LOCK_FILE: &str = "/tmp/.make.command.lck";

/// Who holds the test lock, as far as one worktree is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LockHolder {
    /// Nobody: the file is missing or its lock is free.
    Nobody,
    /// A test run started in another directory, or one that did not say where it was started.
    Elsewhere,
    /// A test run started in the worktree or in a directory below it.
    Worktree,
}

/// Why the holder of the test lock cannot be told.
#[derive(Debug, thiserror::Error)]
pub(super) enum TestLockError {
    /// The lock file exists and cannot be opened.
    #[error("cannot open the test lock file {path:?}")]
    Open {
        /// The lock file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The lock cannot be tried.
    #[error("cannot try the test lock {path:?}")]
    Lock {
        /// The lock file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The holder written in the lock file cannot be read.
    #[error("cannot read the holder in the test lock file {path:?}")]
    Read {
        /// The lock file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
}

/// Who holds the test lock in `lock_file`, for the worktree `worktree`.
///
/// # Errors
/// [`TestLockError`] when the file exists and cannot be opened, tried or read.
pub(super) fn holder_of(lock_file: &Path, worktree: &Path) -> Result<LockHolder, TestLockError> {
    let mut file = match File::open(lock_file) {
        Ok(file) => file,
        // No test run has ever taken the lock on this machine.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(LockHolder::Nobody),
        Err(source) => {
            return Err(TestLockError::Open {
                path: lock_file.to_path_buf(),
                source,
            });
        }
    };
    match file.try_lock_shared() {
        // Free. The shared lock is let go when `file` is dropped at the end of this call.
        Ok(()) => return Ok(LockHolder::Nobody),
        Err(TryLockError::WouldBlock) => {}
        Err(TryLockError::Error(source)) => {
            return Err(TestLockError::Lock {
                path: lock_file.to_path_buf(),
                source,
            });
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|source| TestLockError::Read {
        path: lock_file.to_path_buf(),
        source,
    })?;
    let started_in = text.lines().find_map(|line| line.strip_prefix("dir="));
    Ok(match started_in {
        Some(dir) if Path::new(dir).starts_with(worktree) => LockHolder::Worktree,
        Some(_) | None => LockHolder::Elsewhere,
    })
}

#[cfg(test)]
mod tests;
