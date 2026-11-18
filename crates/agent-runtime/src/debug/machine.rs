//! Read-only machine and resource observations: load, memory, filesystems, processes, directory sizes.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::num::{NonZeroU64, ParseFloatError, ParseIntError};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use agent_core::debug_summary::{DescendantProcessSummary, FsUsageSummary, MachineSummary, SummarySection};
use agent_core::diagnostic_assessment::{Filesystem, ReadFailure, Usage};
use agent_core::state::Timestamp;
use tokio::io::AsyncReadExt;

use crate::debug_redaction::SecretsCut;
use crate::store::Home;

/// Maximum procfs file or helper output size, in bytes.
const PROC_FILE_LIMIT: u64 = 4 * 1024 * 1024;

/// Errors during machine observation.
#[derive(Debug, thiserror::Error)]
pub(super) enum MachineError {
    /// An I/O error occurred reading a path.
    #[error("cannot read {}", path.display())]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A process exited between directory listing and file read.
    #[error("process at {} disappeared", path.display())]
    ProcessDisappeared {
        /// Path that disappeared.
        path: PathBuf,
    },
    /// Filesystem statistics query failed.
    #[error("cannot stat filesystem {}", path.display())]
    Statvfs {
        /// Filesystem path.
        path: PathBuf,
        /// Underlying errno.
        #[source]
        source: rustix::io::Errno,
    },
    /// A required field was missing from an observation.
    #[error("missing required field {field} in {}", path.display())]
    MissingField {
        /// Path observed.
        path: PathBuf,
        /// Field that was missing.
        field: &'static str,
    },
    /// An integer field could not be parsed.
    #[error("invalid numeric value in {}", path.display())]
    Number {
        /// Path observed.
        path: PathBuf,
        /// Underlying parse error.
        #[source]
        source: ParseIntError,
    },
    /// A floating-point field could not be parsed.
    #[error("invalid float value in {}", path.display())]
    Float {
        /// Path observed.
        path: PathBuf,
        /// Underlying parse error.
        #[source]
        source: ParseFloatError,
    },
    /// An observation had an invalid format or encoding.
    #[error("invalid format in {}: {message}", path.display())]
    InvalidFormat {
        /// Path observed.
        path: PathBuf,
        /// Description of why the format was invalid.
        message: String,
    },
    /// A numeric calculation overflowed.
    #[error("numeric overflow for field {field} in {}", path.display())]
    Overflow {
        /// Path observed.
        path: PathBuf,
        /// Field that overflowed.
        field: &'static str,
    },
    /// A file exceeded the maximum allowed read size.
    #[error("file {} exceeds limit of {limit} bytes", path.display())]
    TooLarge {
        /// Path observed.
        path: PathBuf,
        /// Maximum allowed bytes.
        limit: u64,
    },
    /// A helper command failed.
    #[error("helper command failed")]
    Helper(#[source] crate::process::CaptureError),
    /// A helper command exited with failure status.
    #[error("helper command exited with status {exit:?}")]
    HelperExit {
        /// Exit report.
        exit: crate::process::ExitReport,
    },
    /// UTF-8 decoding error.
    #[error("invalid UTF-8 in {}", path.display())]
    Utf8 {
        /// Path observed.
        path: PathBuf,
        /// Underlying UTF-8 error.
        #[source]
        source: std::string::FromUtf8Error,
    },
    /// An asynchronous blocking task panicked.
    #[error("blocking task failed")]
    Join {
        /// Join error.
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Raw statistics from a statvfs call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StatvfsStats {
    /// Fundamental file system block size.
    pub fragment_size: u64,
    /// Block size.
    pub block_size: u64,
    /// Total blocks on filesystem.
    pub total_blocks: u64,
    /// Free blocks available to unprivileged user.
    pub available_blocks: u64,
    /// Total file nodes (inodes).
    pub total_inodes: u64,
    /// Free inodes available to unprivileged user.
    pub available_inodes: u64,
}

/// Context and parameters for observing host machine metrics.
pub(super) struct MachineParams<'a> {
    /// Agent home directory.
    pub home: &'a Home,
    /// Path to procfs root (usually `/proc`).
    pub procfs: &'a Path,
    /// Temporary directory.
    pub temp_dir: &'a Path,
    /// Daemon PID if known.
    pub daemon_pid: Option<u32>,
    /// Redaction rules.
    pub cut: &'a SecretsCut,
    /// Collection timestamp.
    pub collected_at: Timestamp,
    /// Clock ticks per second for calculating process start time.
    pub clock_ticks: Option<NonZeroU64>,
}

/// Machine observations ready for assessment, summary, and bundle persistence.
#[derive(Debug)]
pub(super) struct MachineObservation {
    /// Summary facts for the Machine section.
    pub(super) summary: MachineSummary,
    /// Unique filesystems for diagnostic assessment.
    pub(super) filesystems: Vec<Filesystem>,
    /// Read failures during collection.
    pub(super) unreadable: Vec<ReadFailure>,
    /// Redacted text artifact for `machine.txt`.
    pub(super) artifact_bytes: Vec<u8>,
}

/// Observed descendant process information.
#[derive(Debug, Clone)]
struct DescendantProcess {
    /// Process ID.
    pid: u32,
    /// Age in seconds from process start time to collection timestamp.
    age_seconds: Option<u64>,
    /// Command line arguments.
    cmdline: Vec<String>,
}

/// Memory and swap facts observed from procfs meminfo.
#[derive(Debug, Default)]
struct MemoryFacts {
    /// Total usable physical memory in bytes.
    total_bytes: Option<u64>,
    /// Available memory in bytes.
    available_bytes: Option<u64>,
    /// Total swap in bytes.
    swap_total: Option<u64>,
    /// Free swap in bytes.
    swap_free: Option<u64>,
    /// Whether swap fields were present and readable in meminfo.
    swap_available: bool,
}

/// Diagnostic filesystem statistics for assessment and summary.
#[derive(Debug)]
struct FsFact {
    /// Space usage for assessment.
    space_usage: Option<Usage>,
    /// Inode usage for assessment.
    inode_usage: Option<Usage>,
    /// Summary percentage usage.
    summary: FsUsageSummary,
}

/// Provider of statvfs statistics.
pub(super) trait StatvfsProvider: Send + Sync + 'static {
    /// Queries statvfs for `path`.
    fn statvfs(&self, path: &Path) -> Result<StatvfsStats, rustix::io::Errno>;
}

impl<F> StatvfsProvider for F
where
    F: Fn(&Path) -> Result<StatvfsStats, rustix::io::Errno> + Send + Sync + 'static,
{
    fn statvfs(&self, path: &Path) -> Result<StatvfsStats, rustix::io::Errno> {
        self(path)
    }
}

/// Default production provider delegating to `rustix::fs::statvfs`.
struct DefaultStatvfs;

impl StatvfsProvider for DefaultStatvfs {
    fn statvfs(&self, path: &Path) -> Result<StatvfsStats, rustix::io::Errno> {
        let stat = rustix::fs::statvfs(path)?;
        let fragment_size = stat.f_frsize;
        let block_size = stat.f_bsize;
        let total_blocks = stat.f_blocks;
        let available_blocks = stat.f_bavail;
        let total_inodes = stat.f_files;
        let available_inodes = stat.f_favail;
        Ok(StatvfsStats {
            fragment_size,
            block_size,
            total_blocks,
            available_blocks,
            total_inodes,
            available_inodes,
        })
    }
}

/// Default non-blocking statvfs query running on a blocking thread.
async fn query_statvfs(
    provider: std::sync::Arc<dyn StatvfsProvider>,
    path: PathBuf,
) -> Result<StatvfsStats, MachineError> {
    tokio::task::spawn_blocking(move || {
        provider
            .statvfs(&path)
            .map_err(|source| MachineError::Statvfs { path, source })
    })
    .await
    .map_err(|source| MachineError::Join { source })?
}

/// Bounded read of a procfs file up to `limit` bytes.
async fn read_proc_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, MachineError> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(MachineError::ProcessDisappeared {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(MachineError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut buf = Vec::new();
    let read_limit = limit.saturating_add(1);
    tokio::io::AsyncReadExt::take(&mut file, read_limit)
        .read_to_end(&mut buf)
        .await
        .map_err(|source| MachineError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let bytes_read = u64::try_from(buf.len()).map_err(|_err| MachineError::Overflow {
        path: path.to_path_buf(),
        field: "read_proc_bounded buffer length",
    })?;
    if bytes_read > limit {
        return Err(MachineError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    Ok(buf)
}

/// Bounded read of a procfs file converted to UTF-8 string up to `limit` bytes.
async fn read_proc_string_bounded(path: &Path, limit: u64) -> Result<String, MachineError> {
    let bytes = read_proc_bounded(path, limit).await?;
    String::from_utf8(bytes).map_err(|source| MachineError::Utf8 {
        path: path.to_path_buf(),
        source,
    })
}

/// Collects machine metrics and descendant processes.
pub(super) async fn observe(
    home: &Home,
    procfs: &Path,
    temp_dir: &Path,
    daemon_pid: Option<u32>,
    cut: &SecretsCut,
    now: Timestamp,
) -> MachineObservation {
    let params = MachineParams {
        home,
        procfs,
        temp_dir,
        daemon_pid,
        cut,
        collected_at: now,
        clock_ticks: None,
    };
    observe_with_provider(params, std::sync::Arc::new(DefaultStatvfs)).await
}

/// Collects machine metrics using an injected statvfs provider (for tests).
#[cfg(test)]
pub(super) async fn observe_with_statvfs<F>(params: MachineParams<'_>, statvfs: F) -> MachineObservation
where
    F: Fn(&Path) -> Result<StatvfsStats, rustix::io::Errno> + Send + Sync + 'static,
{
    observe_with_provider(params, std::sync::Arc::new(statvfs)).await
}

/// Queries actual system clock ticks per second using getconf helper.
async fn query_clock_ticks() -> Result<NonZeroU64, MachineError> {
    let mut command = tokio::process::Command::new("getconf");
    command.arg("CLK_TCK");
    let limits = crate::process::CaptureLimits {
        timeout: std::time::Duration::from_secs(2),
        max_bytes: 1024,
    };
    let captured = crate::process::capture(command, limits)
        .await
        .map_err(MachineError::Helper)?;
    if !captured.exit.succeeded() {
        return Err(MachineError::HelperExit { exit: captured.exit });
    }
    let text = String::from_utf8(captured.stdout.bytes).map_err(|source| MachineError::Utf8 {
        path: PathBuf::from("getconf CLK_TCK"),
        source,
    })?;
    let ticks: u64 = text.trim().parse().map_err(|source| MachineError::Number {
        path: PathBuf::from("getconf CLK_TCK"),
        source,
    })?;
    NonZeroU64::new(ticks).ok_or_else(|| MachineError::InvalidFormat {
        path: PathBuf::from("getconf CLK_TCK"),
        message: "zero clock ticks per second".to_owned(),
    })
}

/// Core machine observation pipeline parametrized by statvfs provider.
async fn observe_with_provider(
    params: MachineParams<'_>,
    statvfs_provider: std::sync::Arc<dyn StatvfsProvider>,
) -> MachineObservation {
    let mut unreadable = Vec::new();

    let load_avg = match read_loadavg(params.procfs).await {
        Ok(avg) => Some(avg),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "load average".to_owned(),
                reason: err.to_string(),
            });
            None
        }
    };

    let mem = match read_meminfo(params.procfs).await {
        Ok(mem) => {
            if !mem.swap_available {
                unreadable.push(ReadFailure {
                    section: SummarySection::Machine,
                    part: "swap".to_owned(),
                    reason: "SwapTotal or SwapFree missing in meminfo".to_owned(),
                });
            }
            mem
        }
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "memory".to_owned(),
                reason: err.to_string(),
            });
            MemoryFacts::default()
        }
    };

    let home_path = params.home.root();
    let (filesystems, home_fs, temp_fs, same_fs) =
        observe_filesystems_async(home_path, params.temp_dir, &mut unreadable, statvfs_provider).await;

    let descendants = match observe_descendants(
        params.procfs,
        params.daemon_pid,
        params.cut,
        params.collected_at,
        params.clock_ticks,
        &mut unreadable,
    )
    .await
    {
        Ok(descendants_list) => descendants_list,
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "daemon descendants".to_owned(),
                reason: err.to_string(),
            });
            Vec::new()
        }
    };

    let descendant_summaries: Vec<DescendantProcessSummary> = descendants
        .iter()
        .map(|d| DescendantProcessSummary {
            pid: d.pid,
            age_seconds: d.age_seconds,
            cmdline: d.cmdline.clone(),
        })
        .collect();
    let descendant_pids = descendants.iter().map(|d| d.pid).collect();

    let (tasks_dir_bytes, worktrees_dir_bytes) = observe_dir_sizes(home_path, &mut unreadable).await;

    let summary = MachineSummary {
        load_avg,
        memory_total_bytes: mem.total_bytes,
        memory_available_bytes: mem.available_bytes,
        swap_total_bytes: mem.swap_total,
        swap_free_bytes: mem.swap_free,
        home_fs: home_fs.as_ref().map(|f| f.summary),
        temp_fs: temp_fs.as_ref().map(|f| f.summary),
        daemon_descendants: descendant_summaries,
        daemon_descendant_pids: descendant_pids,
        tasks_dir_bytes,
        worktrees_dir_bytes,
    };

    let ctx = MachineArtifactContext {
        summary: &summary,
        mem: &mem,
        home_path,
        home_fs: home_fs.as_ref(),
        temp_dir: params.temp_dir,
        temp_fs: temp_fs.as_ref(),
        same_fs,
        descendants: &descendants,
        unreadable: &unreadable,
    };

    let artifact_bytes = format_artifact(&ctx).into_bytes();

    MachineObservation {
        summary,
        filesystems,
        unreadable,
        artifact_bytes,
    }
}

async fn observe_single_fs(
    path: &Path,
    label: &str,
    unreadable: &mut Vec<ReadFailure>,
    statvfs_provider: std::sync::Arc<dyn StatvfsProvider>,
) -> Option<FsFact> {
    match query_statvfs(statvfs_provider, path.to_path_buf()).await {
        Ok(st) => match observe_fs_stats(path, st) {
            Ok(fs) => {
                if fs.space_usage.is_none() {
                    unreadable.push(ReadFailure {
                        section: SummarySection::Machine,
                        part: format!("{label} filesystem space"),
                        reason: "filesystem total space is zero".to_owned(),
                    });
                }
                if fs.inode_usage.is_none() {
                    unreadable.push(ReadFailure {
                        section: SummarySection::Machine,
                        part: format!("{label} filesystem inodes"),
                        reason: "filesystem total inodes is zero".to_owned(),
                    });
                }
                Some(fs)
            }
            Err(err) => {
                unreadable.push(ReadFailure {
                    section: SummarySection::Machine,
                    part: format!("filesystem {}", path.display()),
                    reason: err.to_string(),
                });
                None
            }
        },
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("filesystem {}", path.display()),
                reason: err.to_string(),
            });
            None
        }
    }
}

async fn observe_filesystems_async(
    home_path: &Path,
    temp_dir: &Path,
    unreadable: &mut Vec<ReadFailure>,
    statvfs_provider: std::sync::Arc<dyn StatvfsProvider>,
) -> (Vec<Filesystem>, Option<FsFact>, Option<FsFact>, bool) {
    let home_fs = observe_single_fs(home_path, "home", unreadable, std::sync::Arc::clone(&statvfs_provider)).await;
    let temp_fs = observe_single_fs(temp_dir, "temp", unreadable, statvfs_provider).await;

    let mut filesystems = Vec::new();

    let home_meta = tokio::fs::metadata(home_path).await;
    let temp_meta = tokio::fs::metadata(temp_dir).await;

    let same_fs = match (&home_meta, &temp_meta) {
        (Ok(h), Ok(t)) => h.dev() == t.dev(),
        _ => false,
    };

    if let Err(err) = &home_meta
        && err.kind() != std::io::ErrorKind::NotFound
    {
        unreadable.push(ReadFailure {
            section: SummarySection::Machine,
            part: "home filesystem metadata".to_owned(),
            reason: err.to_string(),
        });
    }

    if let Err(err) = &temp_meta
        && err.kind() != std::io::ErrorKind::NotFound
    {
        unreadable.push(ReadFailure {
            section: SummarySection::Machine,
            part: "temp filesystem metadata".to_owned(),
            reason: err.to_string(),
        });
    }

    if let Some(h) = &home_fs {
        filesystems.push(Filesystem {
            path: home_path.to_path_buf(),
            space: h.space_usage,
            inodes: h.inode_usage,
        });
    }

    if !same_fs && let Some(t) = &temp_fs {
        filesystems.push(Filesystem {
            path: temp_dir.to_path_buf(),
            space: t.space_usage,
            inodes: t.inode_usage,
        });
    }

    (filesystems, home_fs, temp_fs, same_fs)
}

fn observe_fs_stats(path: &Path, stat: StatvfsStats) -> Result<FsFact, MachineError> {
    let bsize = if stat.fragment_size > 0 {
        u128::from(stat.fragment_size)
    } else {
        u128::from(stat.block_size)
    };

    if bsize == 0 {
        return Err(MachineError::InvalidFormat {
            path: path.to_path_buf(),
            message: "filesystem block size is zero".to_owned(),
        });
    }

    let total_blocks = u128::from(stat.total_blocks);
    let avail_blocks = u128::from(stat.available_blocks);
    if avail_blocks > total_blocks {
        return Err(MachineError::InvalidFormat {
            path: path.to_path_buf(),
            message: "available space blocks exceed total blocks".to_owned(),
        });
    }

    let total_bytes_u128 = total_blocks.saturating_mul(bsize);
    let avail_bytes_u128 = avail_blocks.saturating_mul(bsize);

    let (space_usage, space_percent) = if total_bytes_u128 > 0 {
        let used_bytes_u128 = total_bytes_u128 - avail_bytes_u128;
        let used_bytes = u64::try_from(used_bytes_u128).map_err(|_err| MachineError::Overflow {
            path: path.to_path_buf(),
            field: "used space bytes",
        })?;
        let total_bytes = u64::try_from(total_bytes_u128).map_err(|_err| MachineError::Overflow {
            path: path.to_path_buf(),
            field: "total space bytes",
        })?;
        let non_zero_total = NonZeroU64::new(total_bytes).ok_or_else(|| MachineError::InvalidFormat {
            path: path.to_path_buf(),
            message: "total space is zero".to_owned(),
        })?;
        let pct_u128 = used_bytes_u128.saturating_mul(100) / total_bytes_u128;
        let pct = u8::try_from(pct_u128).map_err(|_err| MachineError::Overflow {
            path: path.to_path_buf(),
            field: "space percent",
        })?;
        (
            Some(Usage {
                used: used_bytes,
                total: non_zero_total,
            }),
            Some(pct),
        )
    } else {
        (None, None)
    };

    let total_inodes = stat.total_inodes;
    let free_inodes = stat.available_inodes;
    if free_inodes > total_inodes {
        return Err(MachineError::InvalidFormat {
            path: path.to_path_buf(),
            message: "available inodes exceed total inodes".to_owned(),
        });
    }

    let (inode_usage, inode_percent) = if total_inodes > 0 {
        let used_inodes = total_inodes - free_inodes;
        let non_zero_total = NonZeroU64::new(total_inodes).ok_or_else(|| MachineError::InvalidFormat {
            path: path.to_path_buf(),
            message: "total inodes is zero".to_owned(),
        })?;
        let pct_u128 = (u128::from(used_inodes).saturating_mul(100)) / u128::from(total_inodes);
        let pct = u8::try_from(pct_u128).map_err(|_err| MachineError::Overflow {
            path: path.to_path_buf(),
            field: "inode percent",
        })?;
        (
            Some(Usage {
                used: used_inodes,
                total: non_zero_total,
            }),
            Some(pct),
        )
    } else {
        (None, None)
    };

    Ok(FsFact {
        space_usage,
        inode_usage,
        summary: FsUsageSummary {
            space_percent,
            inode_percent,
        },
    })
}

async fn observe_dir_sizes(home_path: &Path, unreadable: &mut Vec<ReadFailure>) -> (Option<u64>, Option<u64>) {
    let tasks_path = home_path.join("tasks");
    let tasks_dir_bytes = match directory_size(&tasks_path).await {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "tasks directory".to_owned(),
                reason: err.to_string(),
            });
            None
        }
    };

    let worktrees_path = home_path.join("worktrees");
    let worktrees_dir_bytes = match directory_size(&worktrees_path).await {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "worktrees directory".to_owned(),
                reason: err.to_string(),
            });
            None
        }
    };

    (tasks_dir_bytes, worktrees_dir_bytes)
}

async fn read_loadavg(procfs: &Path) -> Result<(f64, f64, f64), MachineError> {
    let path = procfs.join("loadavg");
    let content = read_proc_string_bounded(&path, 4096).await?;
    let mut parts = content.split_whitespace();
    let one = parts
        .next()
        .ok_or_else(|| MachineError::MissingField {
            path: path.clone(),
            field: "1-minute load average",
        })?
        .parse::<f64>()
        .map_err(|source| MachineError::Float {
            path: path.clone(),
            source,
        })?;
    let five = parts
        .next()
        .ok_or_else(|| MachineError::MissingField {
            path: path.clone(),
            field: "5-minute load average",
        })?
        .parse::<f64>()
        .map_err(|source| MachineError::Float {
            path: path.clone(),
            source,
        })?;
    let fifteen = parts
        .next()
        .ok_or_else(|| MachineError::MissingField {
            path: path.clone(),
            field: "15-minute load average",
        })?
        .parse::<f64>()
        .map_err(|source| MachineError::Float {
            path: path.clone(),
            source,
        })?;
    Ok((one, five, fifteen))
}

async fn read_meminfo(procfs: &Path) -> Result<MemoryFacts, MachineError> {
    let path = procfs.join("meminfo");
    let content = read_proc_string_bounded(&path, 1024 * 1024).await?;

    let mut mem_total_kb = None;
    let mut mem_avail_kb = None;
    let mut swap_total_kb = None;
    let mut swap_free_kb = None;

    for line in content.lines() {
        if let Some((key, rest)) = line.split_once(':') {
            let key = key.trim();
            let val_str = rest.split_whitespace().next().unwrap_or("");
            match key {
                "MemTotal" => {
                    let val = val_str.parse::<u64>().map_err(|source| MachineError::Number {
                        path: path.clone(),
                        source,
                    })?;
                    mem_total_kb = Some(val);
                }
                "MemAvailable" => {
                    let val = val_str.parse::<u64>().map_err(|source| MachineError::Number {
                        path: path.clone(),
                        source,
                    })?;
                    mem_avail_kb = Some(val);
                }
                "SwapTotal" => {
                    let val = val_str.parse::<u64>().map_err(|source| MachineError::Number {
                        path: path.clone(),
                        source,
                    })?;
                    swap_total_kb = Some(val);
                }
                "SwapFree" => {
                    let val = val_str.parse::<u64>().map_err(|source| MachineError::Number {
                        path: path.clone(),
                        source,
                    })?;
                    swap_free_kb = Some(val);
                }
                _ => {}
            }
        }
    }

    let mem_total = mem_total_kb.ok_or_else(|| MachineError::MissingField {
        path: path.clone(),
        field: "MemTotal",
    })?;
    let mem_avail = mem_avail_kb.ok_or_else(|| MachineError::MissingField {
        path: path.clone(),
        field: "MemAvailable",
    })?;

    let to_bytes = |kb: u64, field: &'static str| -> Result<u64, MachineError> {
        kb.checked_mul(1024).ok_or(MachineError::Overflow {
            path: path.clone(),
            field,
        })
    };

    let total_bytes = Some(to_bytes(mem_total, "MemTotal")?);
    let available_bytes = Some(to_bytes(mem_avail, "MemAvailable")?);
    let swap_total = swap_total_kb.map(|kb| to_bytes(kb, "SwapTotal")).transpose()?;
    let swap_free = swap_free_kb.map(|kb| to_bytes(kb, "SwapFree")).transpose()?;
    let swap_available = swap_total_kb.is_some() && swap_free_kb.is_some();

    Ok(MemoryFacts {
        total_bytes,
        available_bytes,
        swap_total,
        swap_free,
        swap_available,
    })
}

/// Errors computing process age.
#[derive(Debug, thiserror::Error)]
pub(super) enum ProcessAgeError {
    /// Boot time unavailable.
    #[error("boot time unavailable")]
    BootTimeUnavailable,
    /// Clock ticks per second unavailable.
    #[error("clock ticks per second unavailable")]
    ClockTicksUnavailable,
    /// Numeric overflow calculating start seconds.
    #[error("numeric overflow calculating process start time")]
    StartSecOverflow,
    /// Collection timestamp before UNIX epoch.
    #[error("collection timestamp {timestamp} before UNIX epoch")]
    TimestampBeforeEpoch {
        /// Timestamp seconds value.
        timestamp: i64,
    },
}

/// Computes process age in seconds from boot time and start ticks.
fn compute_process_age(
    btime: Option<u64>,
    ticks: Option<NonZeroU64>,
    start_ticks: u64,
    collected_at: Timestamp,
) -> Result<u64, ProcessAgeError> {
    let boot = btime.ok_or(ProcessAgeError::BootTimeUnavailable)?;
    let clk_ticks = ticks.ok_or(ProcessAgeError::ClockTicksUnavailable)?;
    let start_sec = boot
        .checked_add(start_ticks / clk_ticks.get())
        .ok_or(ProcessAgeError::StartSecOverflow)?;
    let now_sec = collected_at.as_utc().unix_timestamp();
    let now_u64 =
        u64::try_from(now_sec).map_err(|_err| ProcessAgeError::TimestampBeforeEpoch { timestamp: now_sec })?;
    Ok(now_u64.saturating_sub(start_sec))
}

/// Traverses descendant processes starting from daemon PID using BFS.
fn traverse_descendants(
    daemon_pid: u32,
    all_procs: &HashMap<u32, DescendantProcess>,
    children_map: &HashMap<u32, Vec<u32>>,
) -> Vec<DescendantProcess> {
    let mut descendants = Vec::new();
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    visited.insert(daemon_pid);

    if let Some(direct_children) = children_map.get(&daemon_pid) {
        for &child in direct_children {
            if visited.insert(child) {
                queue.push_back(child);
            }
        }
    }

    while let Some(current_pid) = queue.pop_front() {
        if let Some(proc) = all_procs.get(&current_pid) {
            descendants.push(proc.clone());
        }
        if let Some(children) = children_map.get(&current_pid) {
            for &child in children {
                if visited.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }

    descendants.sort_by_key(|p| p.pid);
    descendants
}

struct ProcessReadContext<'a> {
    procfs: &'a Path,
    btime: Option<u64>,
    ticks: Option<NonZeroU64>,
    cut: &'a SecretsCut,
    collected_at: Timestamp,
}

async fn read_single_process(
    ctx: &ProcessReadContext<'_>,
    pid_str: &str,
    pid: u32,
    unreadable: &mut Vec<ReadFailure>,
) -> Option<(u32, DescendantProcess)> {
    let pid_dir = ctx.procfs.join(pid_str);
    let stat_content = match read_proc_string_bounded(&pid_dir.join("stat"), 65536).await {
        Ok(s) => s,
        Err(MachineError::ProcessDisappeared { .. }) => return None,
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("process {pid} stat"),
                reason: err.to_string(),
            });
            return None;
        }
    };

    let Some((parent_pid, start_ticks)) = parse_ppid_and_start_ticks(&stat_content) else {
        unreadable.push(ReadFailure {
            section: SummarySection::Machine,
            part: format!("process {pid} stat"),
            reason: "cannot parse parent PID and start ticks from stat".to_owned(),
        });
        return None;
    };

    let cmdline = match read_proc_bounded(&pid_dir.join("cmdline"), PROC_FILE_LIMIT).await {
        Ok(bytes) => parse_and_redact_cmdline(&bytes, ctx.cut),
        Err(MachineError::ProcessDisappeared { .. }) => return None,
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("process {pid} cmdline"),
                reason: err.to_string(),
            });
            Vec::new()
        }
    };

    let age_seconds = match compute_process_age(ctx.btime, ctx.ticks, start_ticks, ctx.collected_at) {
        Ok(age) => Some(age),
        Err(err) => {
            if ctx.btime.is_some() && ctx.ticks.is_some() {
                unreadable.push(ReadFailure {
                    section: SummarySection::Machine,
                    part: format!("process {pid} age"),
                    reason: err.to_string(),
                });
            }
            None
        }
    };

    Some((
        parent_pid,
        DescendantProcess {
            pid,
            age_seconds,
            cmdline,
        },
    ))
}

async fn observe_descendants(
    procfs: &Path,
    daemon_pid: Option<u32>,
    cut: &SecretsCut,
    collected_at: Timestamp,
    clock_ticks: Option<NonZeroU64>,
    unreadable: &mut Vec<ReadFailure>,
) -> Result<Vec<DescendantProcess>, MachineError> {
    let Some(target_daemon_pid) = daemon_pid else {
        return Ok(Vec::new());
    };

    let (ticks, ticks_err) = match clock_ticks {
        Some(t) => (Some(t), None),
        None => match query_clock_ticks().await {
            Ok(t) => (Some(t), None),
            Err(err) => (None, Some(err)),
        },
    };
    if let Some(err) = ticks_err {
        unreadable.push(ReadFailure {
            section: SummarySection::Machine,
            part: "process age".to_owned(),
            reason: format!("clock ticks per second unavailable: {err}"),
        });
    }

    let btime = match read_btime(procfs).await {
        Ok(b) => Some(b),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "process age".to_owned(),
                reason: format!("cannot read boot time: {err}"),
            });
            None
        }
    };

    let mut read_dir = tokio::fs::read_dir(procfs).await.map_err(|source| MachineError::Io {
        path: procfs.to_path_buf(),
        source,
    })?;

    let mut all_procs = HashMap::new();
    let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();
    let ctx = ProcessReadContext {
        procfs,
        btime,
        ticks,
        cut,
        collected_at,
    };

    while let Some(entry) = read_dir.next_entry().await.map_err(|source| MachineError::Io {
        path: procfs.to_path_buf(),
        source,
    })? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = name_str.parse::<u32>() else {
            continue;
        };

        if let Some((parent_pid, proc)) = read_single_process(&ctx, &name_str, pid, unreadable).await {
            all_procs.insert(pid, proc);
            children_map.entry(parent_pid).or_default().push(pid);
        }
    }

    Ok(traverse_descendants(target_daemon_pid, &all_procs, &children_map))
}

async fn read_btime(procfs: &Path) -> Result<u64, MachineError> {
    let stat_path = procfs.join("stat");
    let text = read_proc_string_bounded(&stat_path, 1024 * 1024).await?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("btime ") {
            let btime = rest.trim().parse::<u64>().map_err(|source| MachineError::Number {
                path: stat_path.clone(),
                source,
            })?;
            return Ok(btime);
        }
    }
    Err(MachineError::MissingField {
        path: stat_path,
        field: "btime",
    })
}

fn parse_ppid_and_start_ticks(stat: &str) -> Option<(u32, u64)> {
    let (_, after_paren) = stat.rsplit_once(')')?;
    let mut parts = after_paren.split_whitespace();
    let _state = parts.next()?;
    let ppid_str = parts.next()?;
    let ppid = ppid_str.parse::<u32>().ok()?;

    let start_ticks_str = parts.nth(17)?;
    let start_ticks = start_ticks_str.parse::<u64>().ok()?;

    Some((ppid, start_ticks))
}

fn parse_and_redact_cmdline(raw_bytes: &[u8], cut: &SecretsCut) -> Vec<String> {
    raw_bytes
        .split(|&b| b == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| {
            let text = String::from_utf8_lossy(arg);
            cut.redact_text(&text)
        })
        .collect()
}

/// Maximum number of directory entries to inspect when calculating directory sizes.
const MAX_DIRECTORY_SIZE_ENTRIES: usize = 100_000;

async fn directory_size(path: &Path) -> Result<u64, MachineError> {
    directory_size_bounded(path, MAX_DIRECTORY_SIZE_ENTRIES).await
}

async fn directory_size_bounded(path: &Path, max_entries: usize) -> Result<u64, MachineError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => {
            if !meta.is_dir() {
                return Ok(meta.len());
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(source) => {
            return Err(MachineError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    let mut total_bytes = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    let mut entries_count: usize = 0;

    while let Some(dir) = pending.pop() {
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(MachineError::Io { path: dir, source });
            }
        };

        while let Some(entry) = read_dir.next_entry().await.map_err(|source| MachineError::Io {
            path: dir.clone(),
            source,
        })? {
            entries_count = entries_count.checked_add(1).ok_or_else(|| MachineError::Overflow {
                path: path.to_path_buf(),
                field: "directory entry count",
            })?;
            if entries_count > max_entries {
                let limit = u64::try_from(max_entries).map_err(|_err| MachineError::Overflow {
                    path: path.to_path_buf(),
                    field: "directory size entry limit",
                })?;
                return Err(MachineError::TooLarge {
                    path: path.to_path_buf(),
                    limit,
                });
            }

            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(MachineError::Io {
                        path: entry.path(),
                        source,
                    });
                }
            };

            if meta.is_symlink() {
                continue;
            }

            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total_bytes = total_bytes
                    .checked_add(meta.len())
                    .ok_or_else(|| MachineError::Overflow {
                        path: path.to_path_buf(),
                        field: "directory total bytes",
                    })?;
            }
        }
    }

    Ok(total_bytes)
}

/// Parameters needed to format the machine artifact document.
struct MachineArtifactContext<'a> {
    /// Summary of machine metrics.
    summary: &'a MachineSummary,
    /// Memory and swap facts.
    mem: &'a MemoryFacts,
    /// Home directory path.
    home_path: &'a Path,
    /// Home filesystem facts, if available.
    home_fs: Option<&'a FsFact>,
    /// Temporary directory path.
    temp_dir: &'a Path,
    /// Temporary filesystem facts, if available.
    temp_fs: Option<&'a FsFact>,
    /// Whether home and temp reside on the same filesystem.
    same_fs: bool,
    /// Collected daemon descendant processes.
    descendants: &'a [DescendantProcess],
    /// Read failures recorded during observation.
    unreadable: &'a [ReadFailure],
}

fn format_artifact(ctx: &MachineArtifactContext<'_>) -> String {
    let mut out = String::new();
    let mut append = |line: &str| {
        out.push_str(line);
        out.push('\n');
    };

    append("## Machine Metrics and Resources");
    append("");

    append("### Load Average");
    if let Some((one, five, fifteen)) = ctx.summary.load_avg {
        append(&format!("- Load average: {one:.2}, {five:.2}, {fifteen:.2}"));
    } else {
        append("- Load average: unreadable");
    }
    append("");

    append("### Memory and Swap");
    if let (Some(avail), Some(total)) = (ctx.mem.available_bytes, ctx.mem.total_bytes) {
        append(&format!("- Memory: {avail} / {total} bytes available"));
    } else {
        append("- Memory: unknown");
    }
    if let (Some(swap_free), Some(swap_tot)) = (ctx.mem.swap_free, ctx.mem.swap_total) {
        append(&format!("- Swap: {swap_free} / {swap_tot} bytes free"));
    } else {
        append("- Swap: unknown");
    }
    append("");

    append("### Filesystems");
    append(&format!("- Home directory: {}", ctx.home_path.display()));
    if let Some(h) = ctx.home_fs {
        let space_str = match h.summary.space_percent {
            Some(p) => format!("{p}% used"),
            None => "unknown".to_owned(),
        };
        let inode_str = match h.summary.inode_percent {
            Some(p) => format!("{p}% used"),
            None => "unknown".to_owned(),
        };
        append(&format!("  Space: {space_str}; Inodes: {inode_str}"));
    }
    append(&format!("- Temporary directory: {}", ctx.temp_dir.display()));
    if ctx.same_fs {
        append("  (shares filesystem with home directory)");
    } else if let Some(t) = ctx.temp_fs {
        let space_str = match t.summary.space_percent {
            Some(p) => format!("{p}% used"),
            None => "unknown".to_owned(),
        };
        let inode_str = match t.summary.inode_percent {
            Some(p) => format!("{p}% used"),
            None => "unknown".to_owned(),
        };
        append(&format!("  Space: {space_str}; Inodes: {inode_str}"));
    }
    append("");

    append("### Directory Sizes");
    if let Some(bytes) = ctx.summary.tasks_dir_bytes {
        append(&format!("- Tasks directory: {bytes} bytes"));
    }
    if let Some(bytes) = ctx.summary.worktrees_dir_bytes {
        append(&format!("- Worktrees directory: {bytes} bytes"));
    }
    append("");

    let has_descendant_failure = ctx
        .unreadable
        .iter()
        .any(|f| f.section == SummarySection::Machine && f.part.contains("descendant"));

    append(&format!("### Daemon Descendant Processes ({})", ctx.descendants.len()));
    if has_descendant_failure {
        append("Descendant processes could not be read completely.");
    } else if ctx.descendants.is_empty() {
        append("No descendant processes found.");
    } else {
        for proc in ctx.descendants {
            let age_str = match proc.age_seconds {
                Some(age) => format!("{age}s"),
                None => "unknown".to_owned(),
            };
            let cmdline = proc.cmdline.join(" ");
            append(&format!("- PID {}: age {age_str}", proc.pid));
            append(&format!("  Command: {cmdline}"));
        }
    }
    append("");

    if !ctx.unreadable.is_empty() {
        append("### Unreadable Observations");
        for failure in ctx.unreadable {
            append(&format!("- {}: {}", failure.part, failure.reason));
        }
        append("");
    }

    out
}
