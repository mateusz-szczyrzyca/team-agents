//! Read-only machine and resource observations: load, memory, filesystems, processes, directory sizes.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::num::NonZeroU64;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use agent_core::debug_summary::{FsUsageSummary, MachineSummary, SummarySection};
use agent_core::diagnostic_assessment::{Filesystem, ReadFailure, Usage};
use agent_core::state::Timestamp;

use crate::debug_redaction::SecretsCut;
use crate::store::Home;

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
    /// Total inodes.
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

/// Collects machine metrics using an injected statvfs provider.
pub(super) async fn observe_with_statvfs<F>(params: MachineParams<'_>, _statvfs: F) -> MachineObservation
where
    F: Fn(&Path) -> Result<StatvfsStats, rustix::io::Errno> + Send + Sync + 'static,
{
    observe(
        params.home,
        params.procfs,
        params.temp_dir,
        params.daemon_pid,
        params.cut,
        params.collected_at,
    )
    .await
}

/// Collects machine metrics and descendant processes.
pub(super) async fn observe(
    home: &Home,
    procfs: &Path,
    temp_dir: &Path,
    daemon_pid: Option<u32>,
    cut: &SecretsCut,
    _now: Timestamp,
) -> MachineObservation {
    let mut unreadable = Vec::new();

    let load_avg = match read_loadavg(procfs).await {
        Ok(avg) => Some(avg),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "load average".to_owned(),
                reason: err,
            });
            None
        }
    };

    let mem = match read_meminfo(procfs).await {
        Ok(mem) => mem,
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "memory".to_owned(),
                reason: err,
            });
            MemoryFacts::default()
        }
    };

    let home_path = home.root();
    let (filesystems, home_fs, temp_fs, same_fs) = observe_filesystems(home_path, temp_dir, &mut unreadable).await;

    let descendants = match observe_descendants(procfs, daemon_pid, cut).await {
        Ok(descendants_list) => descendants_list,
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: "daemon descendants".to_owned(),
                reason: err,
            });
            Vec::new()
        }
    };
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
        daemon_descendants: vec![],
        daemon_descendant_pids: descendant_pids,
        tasks_dir_bytes,
        worktrees_dir_bytes,
    };

    let artifact_bytes = format_artifact(
        &summary,
        &mem,
        home_path,
        home_fs.as_ref(),
        temp_dir,
        temp_fs.as_ref(),
        same_fs,
        &descendants,
        &unreadable,
    )
    .unwrap_or_default()
    .into_bytes();

    MachineObservation {
        summary,
        filesystems,
        unreadable,
        artifact_bytes,
    }
}

async fn observe_filesystems(
    home_path: &Path,
    temp_dir: &Path,
    unreadable: &mut Vec<ReadFailure>,
) -> (Vec<Filesystem>, Option<FsFact>, Option<FsFact>, bool) {
    let home_fs = match observe_fs(home_path) {
        Ok(fs) => Some(fs),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("filesystem {}", home_path.display()),
                reason: err,
            });
            None
        }
    };

    let temp_fs = match observe_fs(temp_dir) {
        Ok(fs) => Some(fs),
        Err(err) => {
            unreadable.push(ReadFailure {
                section: SummarySection::Machine,
                part: format!("filesystem {}", temp_dir.display()),
                reason: err,
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

async fn read_loadavg(procfs: &Path) -> Result<(f64, f64, f64), String> {
    let path = procfs.join("loadavg");
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut parts = content.split_whitespace();
    let one = parts
        .next()
        .ok_or_else(|| "missing 1-minute load average".to_owned())?
        .parse::<f64>()
        .map_err(|e| format!("invalid 1-minute load average: {e}"))?;
    let five = parts
        .next()
        .ok_or_else(|| "missing 5-minute load average".to_owned())?
        .parse::<f64>()
        .map_err(|e| format!("invalid 5-minute load average: {e}"))?;
    let fifteen = parts
        .next()
        .ok_or_else(|| "missing 15-minute load average".to_owned())?
        .parse::<f64>()
        .map_err(|e| format!("invalid 15-minute load average: {e}"))?;
    Ok((one, five, fifteen))
}

async fn read_meminfo(procfs: &Path) -> Result<MemoryFacts, String> {
    let path = procfs.join("meminfo");
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let mut mem_total_kb = None;
    let mut mem_avail_kb = None;
    let mut mem_free_kb = None;
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
                    "MemFree" => mem_free_kb = Some(val),
                    "SwapTotal" => swap_total_kb = Some(val),
                    "SwapFree" => swap_free_kb = Some(val),
                    _ => {}
                }
            }
        }
    }

    let to_bytes = |kb: Option<u64>| kb.and_then(|k| k.checked_mul(1024));

    let total_bytes = to_bytes(mem_total_kb);
    let available_bytes = to_bytes(mem_avail_kb.or(mem_free_kb));
    let swap_total = to_bytes(swap_total_kb);
    let swap_free = to_bytes(swap_free_kb);

    if total_bytes.is_none() && available_bytes.is_none() {
        return Err("missing MemTotal and MemAvailable in meminfo".to_owned());
    }

    Ok(MemoryFacts {
        total_bytes,
        available_bytes,
        swap_total,
        swap_free,
    })
}

fn observe_fs(path: &Path) -> Result<FsFact, String> {
    let stat = rustix::fs::statvfs(path).map_err(|e| format!("statvfs failed: {e}"))?;

    let bsize = if stat.f_frsize > 0 {
        u128::from(stat.f_frsize)
    } else {
        u128::from(stat.f_bsize)
    };

    let total_blocks = u128::from(stat.f_blocks);
    let avail_blocks = u128::from(stat.f_bavail);
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

    let total_inodes = stat.f_files;
    let free_inodes = stat.f_favail;
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

    Ok(FsFact {
        space_usage,
        inode_usage,
        summary: FsUsageSummary {
            space_percent,
            inode_percent,
        },
    })
}

async fn observe_descendants(
    procfs: &Path,
    daemon_pid: Option<u32>,
    cut: &SecretsCut,
) -> Result<Vec<DescendantProcess>, String> {
    let Some(target_daemon_pid) = daemon_pid else {
        return Ok(Vec::new());
    };

    let mut read_dir = match tokio::fs::read_dir(procfs).await {
        Ok(rd) => rd,
        Err(e) => return Err(format!("cannot read procfs directory {}: {e}", procfs.display())),
    };

    let mut all_procs = HashMap::new();
    let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(pid) = name_str.parse::<u32>() {
            let pid_dir = procfs.join(&*name_str);
            if let Ok(stat_content) = tokio::fs::read_to_string(pid_dir.join("stat")).await
                && let Some(ppid) = parse_ppid(&stat_content)
            {
                let cmdline = match tokio::fs::read(pid_dir.join("cmdline")).await {
                    Ok(bytes) => parse_and_redact_cmdline(&bytes, cut),
                    Err(_) => Vec::new(),
                };
                all_procs.insert(pid, DescendantProcess { pid, cmdline });
                children_map.entry(ppid).or_default().push(pid);
            }
        }
    }

    let mut descendants = Vec::new();
    let mut queue = VecDeque::new();
    if let Some(direct_children) = children_map.get(&target_daemon_pid) {
        for &child in direct_children {
            queue.push_back(child);
        }
    }

    while let Some(current_pid) = queue.pop_front() {
        if let Some(proc) = all_procs.get(&current_pid) {
            descendants.push(proc.clone());
        }
        if let Some(children) = children_map.get(&current_pid) {
            for &child in children {
                queue.push_back(child);
            }
        }
    }

    descendants.sort_by_key(|p| p.pid);
    Ok(descendants)
}

fn parse_ppid(stat_content: &str) -> Option<u32> {
    let after_comm = stat_content.rfind(')')?;
    let rest = stat_content.get(after_comm.saturating_add(1)..)?;
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let ppid_str = fields.next()?;
    ppid_str.parse::<u32>().ok()
}

fn parse_and_redact_cmdline(bytes: &[u8], cut: &SecretsCut) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let text = String::from_utf8_lossy(part);
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

        while let Ok(Some(entry)) = read_dir.next_entry().await {
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

#[expect(clippy::too_many_arguments, reason = "internal artifact renderer")]
fn format_artifact(
    summary: &MachineSummary,
    mem: &MemoryFacts,
    home_path: &Path,
    home_fs: Option<&FsFact>,
    temp_dir: &Path,
    temp_fs: Option<&FsFact>,
    same_fs: bool,
    descendants: &[DescendantProcess],
    unreadable: &[ReadFailure],
) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    out.push_str("# Machine and Resources\n\n");

    if let Some((one, five, fifteen)) = summary.load_avg {
        writeln!(out, "Load average: {one:.2}, {five:.2}, {fifteen:.2}")?;
    } else {
        out.push_str("Load average: unknown\n");
    }

    if let (Some(avail), Some(total)) = (mem.available_bytes, mem.total_bytes) {
        writeln!(out, "Memory: {avail} / {total} bytes available")?;
    } else {
        out.push_str("Memory: unknown\n");
    }

    if let (Some(free), Some(total)) = (mem.swap_free, mem.swap_total) {
        writeln!(out, "Swap: {free} / {total} bytes free")?;
    } else {
        out.push_str("Swap: unknown\n");
    }

    out.push_str("\n## Filesystems\n");
    if let Some(h) = home_fs {
        writeln!(
            out,
            "Home (\"{}\"): space {}%, inodes {}%",
            home_path.display(),
            h.summary.space_percent,
            h.summary.inode_percent
        )?;
    }
    if same_fs {
        out.push_str("Temp: shared with home filesystem\n");
    } else if let Some(t) = temp_fs {
        writeln!(
            out,
            "Temp (\"{}\"): space {}%, inodes {}%",
            temp_dir.display(),
            t.summary.space_percent,
            t.summary.inode_percent
        )?;
    }

    out.push_str("\n## Daemon Descendants\n");
    if descendants.is_empty() {
        out.push_str("None\n");
    } else {
        for d in descendants {
            let cmd = d.cmdline.join(" ");
            writeln!(out, "- PID {}: {}", d.pid, cmd)?;
        }
    }

    out.push_str("\n## Directory Sizes\n");
    if let Some(bytes) = summary.tasks_dir_bytes {
        writeln!(out, "Tasks: {bytes} bytes")?;
    } else {
        out.push_str("Tasks: unknown\n");
    }
    if let Some(bytes) = summary.worktrees_dir_bytes {
        writeln!(out, "Worktrees: {bytes} bytes")?;
    } else {
        out.push_str("Worktrees: unknown\n");
    }

    if !unreadable.is_empty() {
        out.push_str("\n## Unreadable Parts\n");
        for u in unreadable {
            writeln!(out, "- {}: {}", u.part, u.reason)?;
        }
    }

    Ok(out)
}
