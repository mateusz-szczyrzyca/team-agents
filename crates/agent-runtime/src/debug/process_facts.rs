//! Read-only Linux daemon observations, selected by ownership of the home's lock.

use std::num::{NonZeroU64, ParseIntError};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process::{CaptureLimits, ExitReport, capture};
use crate::store::Home;
use agent_core::debug_summary::DaemonSummary;
use agent_core::state::Timestamp;
use tokio::io::AsyncReadExt;

/// Maximum procfs file or helper output size, in bytes.
const LIMIT: u64 = 4 * 1024 * 1024;

/// Fresh procfs lock-table observations before concluding that no daemon owns the home.
const LOCK_OBSERVATIONS: usize = 3;

/// Why daemon process observation failed.
#[derive(Debug, thiserror::Error)]
pub(super) enum ProcessError {
    /// A required process file could not be read.
    #[error("cannot read daemon observation {}", path.display())]
    Io {
        /// Failed path.
        path: PathBuf,
        /// Original operating system error.
        source: std::io::Error,
    },
    /// The operating system returned an incomplete observation.
    #[error("invalid daemon observation: {0}")]
    Format(&'static str),
    /// A numeric field could not be decoded.
    #[error("invalid numeric daemon observation")]
    Number(#[source] ParseIntError),
    /// Process bytes are not valid text.
    #[error("daemon observation is not UTF-8")]
    Utf8(#[source] std::str::Utf8Error),
    /// A helper could not complete within its bounds.
    #[error("cannot capture daemon helper")]
    Capture(#[source] crate::process::CaptureError),
    /// A helper returned a failure exit code.
    #[error("daemon helper exited with {0}")]
    Exit(i32),
    /// A helper was terminated by a signal.
    #[error("daemon helper terminated by signal {0}")]
    Signal(i32),
    /// An observation exceeded its byte budget.
    #[error("daemon observation exceeds byte limit")]
    TooLarge,
    /// A numeric observation exceeds the platform range.
    #[error("daemon numeric observation is out of range")]
    Range(#[source] std::num::TryFromIntError),
    /// The start time is outside the representable calendar.
    #[error("daemon start time is outside the supported calendar")]
    Time(#[source] time::error::ComponentRange),
    /// The start year is outside the application's range.
    #[error("invalid daemon start time")]
    Timestamp(#[source] agent_core::state::InvalidTimestamp),
}

async fn bytes(path: &Path) -> Result<Vec<u8>, ProcessError> {
    let file = tokio::fs::File::open(path).await.map_err(|source| ProcessError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut data = Vec::new();
    file.take(LIMIT.saturating_add(1))
        .read_to_end(&mut data)
        .await
        .map_err(|source| ProcessError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if data.len() > 4 * 1024 * 1024 {
        return Err(ProcessError::TooLarge);
    }
    Ok(data)
}

async fn helper(program: &Path, argument: &str) -> Result<String, ProcessError> {
    let mut command = tokio::process::Command::new(program);
    command.arg(argument);
    let result = capture(
        command,
        CaptureLimits {
            timeout: Duration::from_secs(5),
            max_bytes: LIMIT,
        },
    )
    .await
    .map_err(ProcessError::Capture)?;
    if result.stdout.dropped != 0 || result.stderr.dropped != 0 {
        return Err(ProcessError::TooLarge);
    }
    match result.exit {
        ExitReport::Exited { code: 0 } => Ok(std::str::from_utf8(&result.stdout.bytes)
            .map_err(ProcessError::Utf8)?
            .trim()
            .to_owned()),
        ExitReport::Exited { code } => Err(ProcessError::Exit(code)),
        ExitReport::Signaled { signal } => Err(ProcessError::Signal(signal)),
    }
}

/// Observes the process holding this home's lock without changing the home.
///
/// # Errors
/// Returns [`ProcessError`] for unreadable, malformed or oversized observations.
pub(super) async fn observe(home: &Home, procfs: &Path) -> Result<Option<DaemonSummary>, ProcessError> {
    // Do not run a helper when no daemon exists; absence needs only lock metadata.
    if lock_metadata(home).await?.is_none() {
        return Ok(None);
    }
    let ticks = helper(Path::new("getconf"), "CLK_TCK")
        .await?
        .parse::<u64>()
        .map_err(ProcessError::Number)?;
    observe_with_ticks(
        home,
        procfs,
        NonZeroU64::new(ticks).ok_or(ProcessError::Format("zero clock ticks per second"))?,
    )
    .await
}

async fn lock_metadata(home: &Home) -> Result<Option<std::fs::Metadata>, ProcessError> {
    match tokio::fs::metadata(home.lock_file()).await {
        Ok(meta) => Ok(Some(meta)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ProcessError::Io {
            path: home.lock_file(),
            source,
        }),
    }
}

async fn observe_with_ticks(
    home: &Home,
    procfs: &Path,
    ticks: NonZeroU64,
) -> Result<Option<DaemonSummary>, ProcessError> {
    let Some(meta) = lock_metadata(home).await? else {
        return Ok(None);
    };
    let locks_path = procfs.join("locks");
    let Some(pid) = observed_lock_owner(&meta, async || bytes(&locks_path).await).await? else {
        return Ok(None);
    };
    let process = procfs.join(pid.to_string());
    let stat = bytes(&process.join("stat")).await?;
    let boot = bytes(&procfs.join("stat")).await?;
    let started_at = start_time(
        std::str::from_utf8(&stat).map_err(ProcessError::Utf8)?,
        std::str::from_utf8(&boot).map_err(ProcessError::Utf8)?,
        ticks,
    )?;
    let cmd = bytes(&process.join("cmdline")).await?;
    let env = bytes(&process.join("environ")).await?;
    let exe = tokio::fs::read_link(process.join("exe"))
        .await
        .map_err(|source| ProcessError::Io {
            path: process.join("exe"),
            source,
        })?;
    let running = tokio::fs::metadata(process.join("exe"))
        .await
        .map_err(|source| ProcessError::Io {
            path: process.join("exe"),
            source,
        })?;
    let exe = disk_path(&exe, running.nlink())?;
    let modified = tokio::fs::metadata(&exe)
        .await
        .and_then(|meta| meta.modified())
        .map_err(|source| ProcessError::Io {
            path: exe.clone(),
            source,
        })?;
    let mut names = std::str::from_utf8(&env)
        .map_err(ProcessError::Utf8)?
        .split('\0')
        .filter(|field| !field.is_empty())
        .map(|field| {
            field
                .split_once('=')
                .map(|(name, _)| name.to_owned())
                .ok_or(ProcessError::Format("environment entry has no name separator"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(Some(DaemonSummary {
        pid,
        started_at: Some(started_at),
        cmdline: std::str::from_utf8(&cmd)
            .map_err(ProcessError::Utf8)?
            .split('\0')
            .filter(|field| !field.is_empty())
            .map(str::to_owned)
            .collect(),
        env_var_names: names,
        exe_path: exe
            .to_str()
            .ok_or(ProcessError::Format("executable path is not UTF-8"))?
            .to_owned(),
        disk_newer_than_process: time::UtcDateTime::from(modified) > started_at.as_utc(),
        version: helper(&process.join("exe"), "--version").await?,
    }))
}

/// Reads the lock table through its I/O boundary before identifying the owner.
async fn observed_lock_owner(
    meta: &std::fs::Metadata,
    mut read: impl AsyncFnMut() -> Result<Vec<u8>, ProcessError>,
) -> Result<Option<u32>, ProcessError> {
    // Unrelated lock churn can move entries between the seq_file reads that
    // compose one procfs observation; a missing owner warrants a fresh table.
    for _ in 0..LOCK_OBSERVATIONS {
        let locks = read().await?;
        if let Some(pid) = lock_owner(std::str::from_utf8(&locks).map_err(ProcessError::Utf8)?, meta)? {
            return Ok(Some(pid));
        }
    }
    Ok(None)
}

fn lock_owner(text: &str, meta: &std::fs::Metadata) -> Result<Option<u32>, ProcessError> {
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        // Blocked requests contain an arrow and are not lock owners.
        let _number = fields.next();
        if fields.next() != Some("FLOCK") {
            continue;
        }
        if fields.next() != Some("ADVISORY") || fields.next() != Some("WRITE") {
            continue;
        }
        let pid = fields.next().ok_or(ProcessError::Format("lock owner is missing"))?;
        let device = fields.next().ok_or(ProcessError::Format("lock device is missing"))?;
        let mut parts = device.split(':');
        let major = u32::from_str_radix(parts.next().ok_or(ProcessError::Format("device major is missing"))?, 16)
            .map_err(ProcessError::Number)?;
        let minor = u32::from_str_radix(parts.next().ok_or(ProcessError::Format("device minor is missing"))?, 16)
            .map_err(ProcessError::Number)?;
        let inode = parts
            .next()
            .ok_or(ProcessError::Format("lock inode is missing"))?
            .parse::<u64>()
            .map_err(ProcessError::Number)?;
        if (major, minor, inode) == (rustix::fs::major(meta.dev()), rustix::fs::minor(meta.dev()), meta.ino()) {
            return Ok(Some(pid.parse().map_err(ProcessError::Number)?));
        }
    }
    Ok(None)
}

fn start_time(stat: &str, boot: &str, ticks: NonZeroU64) -> Result<Timestamp, ProcessError> {
    // comm may contain spaces and closing parentheses; the final ')' ends it.
    let (_, after_name) = stat
        .rsplit_once(')')
        .ok_or(ProcessError::Format("process name is missing"))?;
    let start_ticks = after_name
        .split_whitespace()
        .nth(19)
        .ok_or(ProcessError::Format("start ticks are missing"))?
        .parse::<u64>()
        .map_err(ProcessError::Number)?;
    let boot_seconds = boot
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .ok_or(ProcessError::Format("boot time is missing"))?
        .trim()
        .parse::<u64>()
        .map_err(ProcessError::Number)?;
    let seconds = boot_seconds
        .checked_add(start_ticks / ticks.get())
        .ok_or(ProcessError::Format("start time overflow"))?;
    let unix = i64::try_from(seconds).map_err(ProcessError::Range)?;
    Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(unix).map_err(ProcessError::Time)?)
        .map_err(ProcessError::Timestamp)
}

/// Resolves the on-disk executable after a running inode has been replaced.
fn disk_path(link: &Path, links: u64) -> Result<PathBuf, ProcessError> {
    if links != 0 {
        return Ok(link.to_path_buf());
    }
    let name = link
        .as_os_str()
        .as_bytes()
        .strip_suffix(b" (deleted)")
        .ok_or(ProcessError::Format("deleted executable link lacks suffix"))?;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(name.to_vec())))
}

#[cfg(test)]
mod tests;
