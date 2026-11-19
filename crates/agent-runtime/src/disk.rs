//! Free disk space measurement, build directory removal, and worktree usage inspection.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::Home;

#[cfg(test)]
mod tests;

/// Why measuring free space failed.
#[derive(Debug, thiserror::Error)]
pub enum FreeSpaceError {
    /// Failed to inspect filesystem via statvfs.
    #[error("cannot measure free space of {path:?}")]
    Statvfs {
        /// The path inspected.
        path: PathBuf,
        /// OS errno.
        source: rustix::io::Errno,
    },
}

/// Why measuring directory size failed.
#[derive(Debug, thiserror::Error)]
pub enum DirectorySizeError {
    /// Reading directory entries failed.
    #[error("cannot read directory {path:?}")]
    ReadDir {
        /// Directory path.
        path: PathBuf,
        /// Underlying IO error.
        source: std::io::Error,
    },
    /// Inspecting file metadata failed.
    #[error("cannot read metadata of {path:?}")]
    Metadata {
        /// File path.
        path: PathBuf,
        /// Underlying IO error.
        source: std::io::Error,
    },
}

/// Why removing a build directory failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildDirError {
    /// The specified worktree is outside the harness worktrees directory.
    #[error("worktree {path:?} is outside the harness worktrees directory")]
    OutsideHome {
        /// The path inspected.
        path: PathBuf,
    },
    /// The target directory is a symbolic link.
    #[error("target path {path:?} is a symbolic link")]
    Symlink {
        /// The path inspected.
        path: PathBuf,
    },
    /// The target path is not a directory.
    #[error("target path {path:?} is not a directory")]
    NotDirectory {
        /// The path inspected.
        path: PathBuf,
    },
    /// Measuring directory size failed before removal.
    #[error("cannot measure target directory {path:?}")]
    Measure {
        /// Target path.
        path: PathBuf,
        /// Cause.
        source: DirectorySizeError,
    },
    /// Removing the directory failed.
    #[error("cannot remove target directory {path:?}")]
    Remove {
        /// Target path.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
}

/// A mechanism for measuring free filesystem space.
pub trait FreeSpace: Send + Sync {
    /// Returns the free space of the filesystem holding `path` in bytes.
    ///
    /// # Errors
    /// [`FreeSpaceError`] if the free space cannot be determined.
    fn free_space_bytes(&self, path: &Path) -> Result<u64, FreeSpaceError>;
}

/// The system filesystem space measurer, using statvfs.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemFreeSpace;

impl FreeSpace for SystemFreeSpace {
    fn free_space_bytes(&self, path: &Path) -> Result<u64, FreeSpaceError> {
        let stat = rustix::fs::statvfs(path).map_err(|source| FreeSpaceError::Statvfs {
            path: path.to_path_buf(),
            source,
        })?;
        let frsize = u64::try_from(stat.f_frsize).unwrap_or(0);
        let bavail = u64::try_from(stat.f_bavail).unwrap_or(0);
        Ok(bavail.saturating_mul(frsize))
    }
}

/// A shared free space measurer.
#[derive(Clone)]
pub struct SharedFreeSpace(Arc<dyn FreeSpace + Send + Sync>);

impl SharedFreeSpace {
    /// Wraps `measurer`.
    #[must_use]
    pub fn new(measurer: impl FreeSpace + Send + Sync + 'static) -> Self {
        Self(Arc::new(measurer))
    }

    /// Creates a shared instance using the system measurer.
    #[must_use]
    pub fn system() -> Self {
        Self::new(SystemFreeSpace)
    }
}

impl fmt::Debug for SharedFreeSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedFreeSpace")
    }
}

impl FreeSpace for SharedFreeSpace {
    fn free_space_bytes(&self, path: &Path) -> Result<u64, FreeSpaceError> {
        self.0.free_space_bytes(path)
    }
}

/// A configurable fake free space measurer for tests.
#[derive(Debug, Default)]
pub struct FakeFreeSpace {
    /// The free space to return, in bytes.
    bytes: AtomicU64,
}

impl FakeFreeSpace {
    /// Creates a fake measurer returning `bytes`.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self {
            bytes: AtomicU64::new(bytes),
        }
    }

    /// Sets the free space in bytes.
    pub fn set(&self, bytes: u64) {
        self.bytes.store(bytes, Ordering::Relaxed);
    }
}

impl FreeSpace for FakeFreeSpace {
    fn free_space_bytes(&self, _path: &Path) -> Result<u64, FreeSpaceError> {
        // Stub for RED: return wrong value 0
        Ok(0)
    }
}

/// Computes the disk usage of `path` by recursively summing file sizes.
///
/// # Errors
/// [`DirectorySizeError`] if files or directories cannot be read.
pub fn directory_size(path: &Path) -> Result<u64, DirectorySizeError> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total: u64 = 0;
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| DirectorySizeError::ReadDir {
            path: dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| DirectorySizeError::ReadDir {
                path: dir.clone(),
                source,
            })?;
            let meta = entry.metadata().map_err(|source| DirectorySizeError::Metadata {
                path: entry.path(),
                source,
            })?;
            if meta.is_dir() {
                dirs.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

/// Represents the disk usage of a single worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeUsage {
    /// Path to the worktree checkout.
    pub path: PathBuf,
    /// Total bytes used by the worktree.
    pub bytes: u64,
}

/// Returns the `count` largest worktrees under `home.worktrees_dir()`, ordered descending by size.
///
/// # Errors
/// [`DirectorySizeError`] if inspecting worktrees fails.
pub fn largest_worktrees(home: &Home, count: usize) -> Result<Vec<WorktreeUsage>, DirectorySizeError> {
    let base = home.worktrees_dir();
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut usages = Vec::new();
    let repo_entries = std::fs::read_dir(&base).map_err(|source| DirectorySizeError::ReadDir {
        path: base.clone(),
        source,
    })?;
    for repo_entry in repo_entries {
        let repo_entry = repo_entry.map_err(|source| DirectorySizeError::ReadDir {
            path: base.clone(),
            source,
        })?;
        if repo_entry.path().is_dir() {
            let task_entries = std::fs::read_dir(repo_entry.path()).map_err(|source| DirectorySizeError::ReadDir {
                path: repo_entry.path(),
                source,
            })?;
            for task_entry in task_entries {
                let task_entry = task_entry.map_err(|source| DirectorySizeError::ReadDir {
                    path: repo_entry.path(),
                    source,
                })?;
                let path = task_entry.path();
                if path.is_dir() {
                    let bytes = directory_size(&path)?;
                    usages.push(WorktreeUsage { path, bytes });
                }
            }
        }
    }
    usages.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    usages.truncate(count);
    Ok(usages)
}

/// Removes the `target/` directory inside `worktree` if present and returns the bytes released.
///
/// Safety guarantees:
/// - Verifies `worktree` is strictly inside `home.worktrees_dir()`.
/// - Never removes a symlink.
///
/// # Errors
/// [`BuildDirError`] if the path is invalid, a symlink, or deletion fails.
pub fn remove_build_directory(home: &Home, worktree: &Path) -> Result<u64, BuildDirError> {
    let worktrees_root = home.worktrees_dir();
    if !worktree.starts_with(&worktrees_root) || worktree == worktrees_root {
        return Err(BuildDirError::OutsideHome {
            path: worktree.to_path_buf(),
        });
    }
    let target = worktree.join("target");
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(source) => return Err(BuildDirError::Remove { path: target, source }),
    };
    if meta.file_type().is_symlink() {
        return Err(BuildDirError::Symlink { path: target });
    }
    if !meta.is_dir() {
        return Err(BuildDirError::NotDirectory { path: target });
    }
    let _bytes = directory_size(&target).map_err(|source| BuildDirError::Measure {
        path: target.clone(),
        source,
    })?;
    std::fs::remove_dir_all(&target).map_err(|source| BuildDirError::Remove {
        path: target.clone(),
        source,
    })?;
    // Stub for RED: return 0 instead of bytes
    Ok(0)
}
