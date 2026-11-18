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

use crate::debug_redaction::SecretsCut;
use crate::store::Home;

/// Maximum procfs file or helper output size, in bytes.
const PROC_FILE_LIMIT: u64 = 4 * 1024 * 1024;

/// Errors during machine observation.
#[derive(Debug, thiserror::Error)]
pub(super) enum MachineError {
    /// An I/O error occurred reading a path.
    #[error("cannot read {path}: {source}")]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Filesystem statistics query failed.
    #[error("cannot stat filesystem {path}: {source}")]
    Statvfs {
        /// Filesystem path.
        path: PathBuf,
        /// Underlying errno.
        source: rustix::io::Errno,
    },
    /// A required field was missing from an observation.
    #[error("missing required field {field} in {path}")]
    MissingField {
        /// Path observed.
        path: PathBuf,
        /// Field that was missing.
        field: &'static str,
    },
    /// An integer field could not be parsed.
    #[error("invalid numeric value in {path}: {source}")]
    Number {
        /// Path observed.
        path: PathBuf,
        /// Underlying parse error.
        source: ParseIntError,
    },
    /// A floating-point field could not be parsed.
    #[error("invalid float value in {path}: {source}")]
    Float {
        /// Path observed.
        path: PathBuf,
        /// Underlying parse error.
        source: ParseFloatError,
    },
    /// An asynchronous blocking task panicked.
    #[error("blocking task failed: {source}")]
    Join {
        /// Join error.
        #[from]
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
    pub clock_ticks: Option<u64>,
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

#[derive(Debug, Clone)]
struct DescendantProcess {
    pid: u32,
    age_seconds: Option<u64>,
    cmdline: Vec<String>,
}

#[derive(Debug, Default)]
struct MemoryFacts {
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    swap_total: Option<u64>,
    swap_free: Option<u64>,
}

#[derive(Debug)]
struct FsFact {
    space_usage: Option<Usage>,
    inode_usage: Option<Usage>,
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

struct DefaultStatvfs;

impl StatvfsProvider for DefaultStatvfs {
    fn statvfs(&self, path: &Path) -> Result<StatvfsStats, rustix::io::Errno> {
        let stat = rustix::fs::statvfs(path)?;
        Ok(StatvfsStats {
            fragment_size: stat.f_frsize as u64,
            block_size: stat.f_bsize as u64,
            total_blocks: stat.f_blocks,
            available_blocks: stat.f_bavail,
            total_inodes: stat.f_files,
            available_inodes: stat.f_favail,
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
    .await?
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
        Ok(mem) => mem,
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

async fn observe_filesystems_async(
    home_path: &Path,
    temp_dir: &Path,
    unreadable: &mut Vec<ReadFailure>,
    statvfs_provider: std::sync::Arc<dyn StatvfsProvider>,
) -> (Vec<Filesystem>, Option<FsFact>, Option<FsFact>, bool) {
    let home_fs = match query_statvfs(std::sync::Arc::clone(&statvfs_provider), home_path.to_path_buf())
        .await
        .map(|st| observe_fs_stats(home_path, st))
    {
        Ok(fs) => Some(fs),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("filesystem {}", home_path.display()),
                reason: err.to_string(),
            });
            None
        }
    };

    let temp_fs = match query_statvfs(statvfs_provider, temp_dir.to_path_buf())
        .await
        .map(|st| observe_fs_stats(temp_dir, st))
    {
        Ok(fs) => Some(fs),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("filesystem {}", temp_dir.display()),
                reason: err.to_string(),
            });
            None
        }
    };

    let mut filesystems = Vec::new();
    let same_fs = match (
        tokio::fs::metadata(home_path).await,
        tokio::fs::metadata(temp_dir).await,
    ) {
        (Ok(h), Ok(t)) => h.dev() == t.dev(),
        _ => false,
    };

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

fn observe_fs_stats(_path: &Path, stat: StatvfsStats) -> FsFact {
    let bsize = if stat.fragment_size > 0 {
        u128::from(stat.fragment_size)
    } else {
        u128::from(stat.block_size)
    };

    let total_blocks = u128::from(stat.total_blocks);
    let avail_blocks = u128::from(stat.available_blocks);
    let total_bytes_u128 = total_blocks.saturating_mul(bsize);
    let avail_bytes_u128 = avail_blocks.saturating_mul(bsize);

    let (used_bytes_u64, total_bytes_u64) = if total_bytes_u128 > 0 {
        let used = total_bytes_u128.saturating_sub(avail_bytes_u128);
        (
            u64::try_from(used).unwrap_or(u64::MAX),
            u64::try_from(total_bytes_u128).ok(),
        )
    } else {
        (0, None)
    };

    let space_usage = total_bytes_u64.and_then(NonZeroU64::new).map(|total| Usage {
        used: used_bytes_u64,
        total,
    });

    let total_inodes = stat.total_inodes;
    let free_inodes = stat.available_inodes;
    let used_inodes = total_inodes.saturating_sub(free_inodes);
    let inode_usage = NonZeroU64::new(total_inodes).map(|total| Usage {
        used: used_inodes,
        total,
    });

    let space_percent = if total_bytes_u128 > 0 {
        let used = total_bytes_u128.saturating_sub(avail_bytes_u128);
        let pct = used.saturating_mul(100) / total_bytes_u128;
        u8::try_from(pct).unwrap_or(255)
    } else {
        0
    };

    let inode_percent = if total_inodes > 0 {
        let pct = (u128::from(used_inodes).saturating_mul(100)) / u128::from(total_inodes);
        u8::try_from(pct).unwrap_or(255)
    } else {
        0
    };

    FsFact {
        space_usage,
        inode_usage,
        summary: FsUsageSummary {
            space_percent,
            inode_percent,
        },
    }
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
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|source| MachineError::Io {
            path: path.clone(),
            source,
        })?;
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
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|source| MachineError::Io {
            path: path.clone(),
            source,
        })?;

    let mut mem_total_kb = None;
    let mut mem_avail_kb = None;
    let mut swap_total_kb = None;
    let mut swap_free_kb = None;

    for line in content.lines() {
        if let Some((key, rest)) = line.split_once(':') {
            let key = key.trim();
            let val_str = rest.split_whitespace().next().unwrap_or("");
            if let Ok(val) = val_str.parse::<u64>() {
                match key {
                    "MemTotal" => mem_total_kb = Some(val),
                    "MemAvailable" => mem_avail_kb = Some(val),
                    "SwapTotal" => swap_total_kb = Some(val),
                    "SwapFree" => swap_free_kb = Some(val),
                    _ => {}
                }
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

    let to_bytes = |kb: u64| kb.checked_mul(1024);

    let total_bytes = to_bytes(mem_total);
    let available_bytes = to_bytes(mem_avail);
    let swap_total = swap_total_kb.and_then(to_bytes);
    let swap_free = swap_free_kb.and_then(to_bytes);

    Ok(MemoryFacts {
        total_bytes,
        available_bytes,
        swap_total,
        swap_free,
    })
}

async fn observe_descendants(
    procfs: &Path,
    daemon_pid: Option<u32>,
    cut: &SecretsCut,
    collected_at: Timestamp,
    clock_ticks: Option<u64>,
) -> Result<Vec<DescendantProcess>, MachineError> {
    let Some(target_daemon_pid) = daemon_pid else {
        return Ok(Vec::new());
    };

    let btime = read_btime(procfs).await.ok();
    let ticks = clock_ticks.unwrap_or(100);

    let mut read_dir = tokio::fs::read_dir(procfs).await.map_err(|source| MachineError::Io {
        path: procfs.to_path_buf(),
        source,
    })?;

    let mut all_procs = HashMap::new();
    let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();

    while let Some(entry) = read_dir.next_entry().await.map_err(|source| MachineError::Io {
        path: procfs.to_path_buf(),
        source,
    })? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(pid) = name_str.parse::<u32>() {
            let pid_dir = procfs.join(&*name_str);
            if let Ok(stat_content) = tokio::fs::read_to_string(pid_dir.join("stat")).await
                && let Some((ppid, start_ticks)) = parse_ppid_and_start_ticks(&stat_content)
            {
                let cmdline = match tokio::fs::read(pid_dir.join("cmdline")).await {
                    Ok(bytes) => parse_and_redact_cmdline(&bytes, cut),
                    Err(_) => Vec::new(),
                };
                let age_seconds = btime.and_then(|boot| {
                    let start_sec = boot.checked_add(start_ticks / ticks)?;
                    let now_sec = collected_at.as_utc().unix_timestamp();
                    let now_sec_u64 = u64::try_from(now_sec).ok()?;
                    Some(now_sec_u64.saturating_sub(start_sec))
                });
                all_procs.insert(
                    pid,
                    DescendantProcess {
                        pid,
                        age_seconds,
                        cmdline,
                    },
                );
                children_map.entry(ppid).or_default().push(pid);
            }
        }
    }

    let mut descendants = Vec::new();
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    visited.insert(target_daemon_pid);

    if let Some(direct_children) = children_map.get(&target_daemon_pid) {
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
    Ok(descendants)
}

async fn read_btime(procfs: &Path) -> Result<u64, MachineError> {
    let stat_path = procfs.join("stat");
    let text = tokio::fs::read_to_string(&stat_path)
        .await
        .map_err(|source| MachineError::Io {
            path: stat_path.clone(),
            source,
        })?;
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

    // starttime is the 22nd field of stat, which is 17th index after state (field 3) and ppid (field 4)
    let start_ticks_str = parts.nth(17)?;
    let start_ticks = start_ticks_str.parse::<u64>().ok()?;

    Some((ppid, start_ticks))
}

fn parse_and_redact_cmdline(raw_bytes: &[u8], cut: &SecretsCut) -> Vec<String> {
    let limit = usize::try_from(PROC_FILE_LIMIT).unwrap_or(raw_bytes.len());
    let bounded = raw_bytes.get(..limit).unwrap_or(raw_bytes);
    bounded
        .split(|&b| b == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| {
            let text = String::from_utf8_lossy(arg);
            cut.redact_text(&text)
        })
        .collect()
}

async fn directory_size(path: &Path) -> Result<u64, std::io::Error> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => {
            if !meta.is_dir() {
                return Ok(meta.len());
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    }

    let mut total_bytes = 0_u64;
    let mut pending = vec![path.to_path_buf()];

    while let Some(dir) = pending.pop() {
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };

        while let Some(entry) = read_dir.next_entry().await? {
            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };

            if meta.is_symlink() {
                continue;
            }

            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total_bytes = total_bytes.saturating_add(meta.len());
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
        append(&format!(
            "  Space: {}% used; Inodes: {}% used",
            h.summary.space_percent, h.summary.inode_percent
        ));
    }
    append(&format!("- Temporary directory: {}", ctx.temp_dir.display()));
    if ctx.same_fs {
        append("  (shares filesystem with home directory)");
    } else if let Some(t) = ctx.temp_fs {
        append(&format!(
            "  Space: {}% used; Inodes: {}% used",
            t.summary.space_percent, t.summary.inode_percent
        ));
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

    append(&format!("### Daemon Descendant Processes ({})", ctx.descendants.len()));
    if ctx.descendants.is_empty() {
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
