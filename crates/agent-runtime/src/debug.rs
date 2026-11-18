//! Read-only diagnostic bundles: observe a team, assess once, redact, then persist.

#[cfg(test)]
mod activity_tests;
mod bus;
mod collect;
mod config;
mod daemon_log;
pub(crate) mod history;
mod machine;
mod notifications;
mod process_facts;
#[cfg(test)]
mod tests;
mod untracked;

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use agent_core::config::{MAX_SECRETS_BYTES, Secrets, SecretsError};
use agent_core::debug_summary::{
    SummaryError, SummaryInput, SummarySection, UntrackedCollection, render_command_summary_with_git_notice,
};
use agent_core::diagnostic_assessment::assess;
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::state::{InvalidTimestamp, State, StateError, Timestamp};

use crate::clock::SystemClock;
use crate::debug_redaction::SecretsCut;
use crate::store::Home;
use crate::text_file::{TextFileError, read_text_file};

/// Observations deferred to the third diagnostic task, never interpreted as empty observations.
const DEFERRED: &[SummarySection] = &[SummarySection::Machine];
/// Maximum state file size in bytes.
const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
/// Default inactivity threshold in minutes.
const STALE_MINUTES: u64 = 120;

/// A round, team or task whose whole team is collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugTarget {
    /// A round of the team.
    Round(RoundId),
    /// The team itself.
    Team(TeamId),
    /// Any task of the team, including reviewers and workers.
    Task(TaskId),
}

/// Options for a diagnostic bundle.
#[derive(Debug, Clone)]
pub struct DebugArgs {
    /// Existing round, team or task.
    pub target: DebugTarget,
    /// Parent of the new directory; defaults to the home's `debug` directory.
    pub out: Option<PathBuf>,
    /// Inactivity threshold in minutes; defaults to 120.
    pub stale_minutes: Option<u64>,
    /// Optional daemon log path; defaults to the home's `daemon.log`.
    pub daemon_log: Option<PathBuf>,
}

/// Paths of a completed, redacted diagnostic bundle.
#[derive(Debug, Clone)]
pub struct DebugResult {
    /// Directory containing all collected artifacts.
    pub bundle_dir: PathBuf,
    /// Bounded human-readable summary.
    pub summary_path: PathBuf,
}

/// Failures that prevent a safe diagnostic bundle from being written.
#[derive(Debug, thiserror::Error)]
pub enum DebugRuntimeError {
    /// An input required before collection could not be read.
    #[error("cannot read diagnostic input {path:?}")]
    Input {
        /// Required input path.
        path: PathBuf,
        /// Bounded text reader's failure.
        source: TextFileError,
    },
    /// A required input does not exist.
    #[error("missing diagnostic input {path:?}")]
    Missing {
        /// Missing state or secrets file.
        path: PathBuf,
    },
    /// Secrets cannot safely be used for redaction.
    #[error("cannot use secrets file")]
    Secrets(#[source] SecretsError),
    /// The saved state violates its schema or invariants.
    #[error("cannot parse diagnostic state")]
    State(#[source] StateError),
    /// No team owns the requested ID.
    #[error("unknown diagnostic target {id}")]
    UnknownTarget {
        /// User-supplied ID.
        id: String,
    },
    /// Collection time is outside the state model's supported range.
    #[error("cannot determine collection time")]
    Clock(#[source] InvalidTimestamp),
    /// State or facts could not be serialized.
    #[error("cannot encode diagnostic facts")]
    Encode(#[source] serde_json::Error),
    /// Required facts remain too large even after compact presentation.
    #[error("cannot render diagnostic summary")]
    Summary(#[source] SummaryError),
    /// The new bundle could not be written.
    #[error("cannot write diagnostic bundle {path:?}")]
    Write {
        /// Destination path.
        path: PathBuf,
        /// Filesystem failure.
        source: std::io::Error,
    },
    /// A participant name cannot be represented in the summary.
    #[error("cannot represent participant name")]
    Name(#[source] agent_core::ids::InvalidName),
    /// A worker performing blocking I/O failed.
    #[error("diagnostic I/O worker failed")]
    Worker(#[source] tokio::task::JoinError),
}

/// Resolves a target using persisted state, without asking a daemon.
///
/// # Errors
/// [`DebugRuntimeError::UnknownTarget`] when no team owns the ID.
pub fn resolve_debug_target(state: &State, target: &DebugTarget) -> Result<TeamId, DebugRuntimeError> {
    let team = match target {
        DebugTarget::Team(id) => state.team(id),
        DebugTarget::Round(id) => state.find_round(id).map(|(team, _round)| team),
        DebugTarget::Task(id) => state.find_task(id).map(|found| found.team),
    };
    team.map(|team| team.id().clone())
        .ok_or_else(|| DebugRuntimeError::UnknownTarget {
            id: match target {
                DebugTarget::Team(id) => id.to_string(),
                DebugTarget::Round(id) => id.to_string(),
                DebugTarget::Task(id) => id.to_string(),
            },
        })
}

/// Reads secrets strictly before any bundle or output parent is created.
///
/// # Errors
/// [`DebugRuntimeError::Input`], [`DebugRuntimeError::Missing`] or [`DebugRuntimeError::Secrets`]
/// for an unreadable, missing, oversized, malformed or insecure secrets file.
pub async fn load_debug_secrets(home: &Home) -> Result<Secrets, DebugRuntimeError> {
    let path = home.secrets_file();
    let file = required_text(&path, MAX_SECRETS_BYTES).await?;
    Secrets::parse(&file.text, file.mode).map_err(DebugRuntimeError::Secrets)
}

/// Loads persisted state without opening the store's writable lock file.
///
/// # Errors
/// [`DebugRuntimeError::Input`], [`DebugRuntimeError::Missing`] or [`DebugRuntimeError::State`].
pub async fn load_debug_state(home: &Home) -> Result<State, DebugRuntimeError> {
    let file = required_text(&home.state_file(), MAX_STATE_BYTES).await?;
    State::from_json(&file.text).map_err(DebugRuntimeError::State)
}

async fn required_text(path: &std::path::Path, limit: usize) -> Result<crate::text_file::TextFile, DebugRuntimeError> {
    read_text_file(path, limit)
        .await
        .map_err(|source| DebugRuntimeError::Input {
            path: path.to_path_buf(),
            source,
        })?
        .ok_or_else(|| DebugRuntimeError::Missing {
            path: path.to_path_buf(),
        })
}

/// Collects a team, makes one pure assessment, and writes only redacted artifacts.
///
/// `status` is the existing status command's output, or its connection/read failure. No daemon
/// mutation or history write is made. Missing optional inputs become assessment read failures.
///
/// # Errors
/// [`DebugRuntimeError`] when required state/secrets, rendering or bundle persistence fails.
pub async fn execute_debug(
    home: &Home,
    args: DebugArgs,
    status: Option<Result<String, String>>,
    out: &mut impl Write,
) -> Result<DebugResult, DebugRuntimeError> {
    execute_debug_at(
        home,
        args,
        status,
        DebugLocation {
            local_offset: time::UtcOffset::UTC,
            checkout: None,
        },
        out,
    )
    .await
}

/// Collects a diagnostic bundle using the caller's machine presentation context.
///
/// # Errors
/// Returns [`DebugRuntimeError`] for required inputs, rendering or persistence failures.
pub async fn execute_debug_at(
    home: &Home,
    args: DebugArgs,
    status: Option<Result<String, String>>,
    location: DebugLocation,
    _out: &mut impl Write,
) -> Result<DebugResult, DebugRuntimeError> {
    let secrets = load_debug_secrets(home).await?;
    let cut = SecretsCut::new(&secrets, std::env::vars_os());
    let state = load_debug_state(home).await?;
    let id = resolve_debug_target(&state, &args.target)?;
    let team = state
        .team(&id)
        .ok_or_else(|| DebugRuntimeError::UnknownTarget { id: id.to_string() })?;
    let now = Timestamp::now(&SystemClock).map_err(DebugRuntimeError::Clock)?;
    let mut collection = collect::Collection::new(
        team,
        now,
        Duration::from_secs(args.stale_minutes.unwrap_or(STALE_MINUTES).saturating_mul(60)),
    )?;
    collection.cut = cut.clone();
    collection.checkout = location.checkout;
    collection.local_offset = location.local_offset;
    collection.observe(home, team, status).await?;
    observe_daemon(&mut collection, home, args.daemon_log.as_deref(), team).await;
    observe_bus(&mut collection, home, team).await;
    observe_notifications(&mut collection, home, team).await;
    observe_config(&mut collection, home, &secrets).await;
    observe_machine(&mut collection, home).await;
    let findings = assess(&collection.assessment);
    collection.summary.anomalies = findings
        .iter()
        .map(|finding| agent_core::debug_summary::SummaryAnomaly {
            message: finding.sentence(),
            command: finding.suggested_command(),
        })
        .collect();
    collection.summary.unreadable_parts = collection
        .assessment
        .unreadable
        .iter()
        .map(|part| agent_core::debug_summary::UnreadablePartSummary {
            section: part.section,
            name: part.part.clone(),
            reason: part.reason.clone(),
        })
        .collect();
    collection.add(
        "facts.txt",
        format!("{:#?}\n{findings:#?}", collection.assessment).into_bytes(),
    );
    let summary = compact_summary_with(
        &collection.summary,
        location.local_offset,
        collection.git_notice.as_deref(),
        |text| {
            // Replacing valid UTF-8 patterns with ASCII preserves valid UTF-8.
            cut.redact_text(text)
        },
    )
    .map_err(DebugRuntimeError::Summary)?;
    collection.add(
        "NOT_COLLECTED.txt",
        b"Not collected in this version: machine metrics; newest untracked file metadata.\n".to_vec(),
    );
    let parent = args.out.unwrap_or_else(|| home.root().join("debug"));
    collect::persist(
        collection,
        cut,
        parent,
        format!("debug-{id}-{}", now.as_utc().unix_timestamp()),
        summary,
    )
    .await
}

fn short(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{prefix}… [full facts in facts.txt]")
    } else {
        prefix
    }
}

#[cfg(test)]
fn compact_summary(input: &SummaryInput) -> Result<String, SummaryError> {
    compact_summary_with(input, time::UtcOffset::UTC, None, str::to_owned)
}

fn compact_summary_with(
    input: &SummaryInput,
    offset: time::UtcOffset,
    git_notice: Option<&str>,
    redact: impl Fn(&str) -> String,
) -> Result<String, SummaryError> {
    match render_command_summary_with_git_notice(
        input,
        offset,
        DEFERRED,
        UntrackedCollection::Deferred,
        &redact,
        git_notice,
    ) {
        Ok(text) => Ok(text),
        Err(SummaryError::RequiredFactsTooLarge { .. }) => {
            let mut compact = input.clone();
            for part in &mut compact.unreadable_parts {
                part.reason = short(&redact(&part.reason), 120);
            }
            for anomaly in &mut compact.anomalies {
                anomaly.message = short(&redact(&anomaly.message), 240);
            }
            for git in &mut compact.git {
                git.commits_over_base.clear();
                git.newest_untracked.clear();
            }
            render_command_summary_with_git_notice(
                &compact,
                offset,
                DEFERRED,
                UntrackedCollection::Deferred,
                &redact,
                git_notice,
            )
        }
        Err(error) => Err(error),
    }
}

/// Records readable daemon facts and marks each failed observation in its own section.
async fn observe_daemon(
    collection: &mut collect::Collection,
    home: &Home,
    daemon_log: Option<&std::path::Path>,
    team: &agent_core::state::Team,
) {
    match process_facts::observe(home, std::path::Path::new("/proc")).await {
        Ok(Some(daemon)) => {
            collection.assessment.daemon = Some(agent_core::diagnostic_assessment::Daemon::Present {
                executable_newer: Some(daemon.disk_newer_than_process),
                version: Some(daemon.version.clone()),
            });
            collection.add_daemon(daemon);
        }
        Ok(None) => {
            collection.assessment.daemon = Some(agent_core::diagnostic_assessment::Daemon::Absent);
            collection.add("daemon-process.txt", b"No process holds the home lock.\n".to_vec());
        }
        Err(error) => collection
            .assessment
            .unreadable
            .push(agent_core::diagnostic_assessment::ReadFailure {
                section: SummarySection::DaemonAndVersions,
                part: "daemon process".to_owned(),
                reason: crate::process::error_chain(&error),
            }),
    }
    match daemon_log::collect(home, daemon_log, team).await {
        Ok(lines) => {
            let text = lines.as_ref().map_or_else(
                || "Daemon log not found.\n".to_owned(),
                |lines| {
                    if lines.is_empty() {
                        String::new()
                    } else {
                        format!("{}\n", lines.join("\n"))
                    }
                },
            );
            collection.add("daemon-log.txt", text.into_bytes());
            collection.summary.daemon_log_lines = lines;
        }
        Err(error) => collection
            .assessment
            .unreadable
            .push(agent_core::diagnostic_assessment::ReadFailure {
                section: SummarySection::DaemonLog,
                part: "daemon log".to_owned(),
                reason: crate::process::error_chain(&error),
            }),
    }
}

/// Records readable bus messages or records an unreadable failure.
async fn observe_bus(collection: &mut collect::Collection, home: &Home, team: &agent_core::state::Team) {
    match bus::collect(home, team, &collection.cut).await {
        Ok(bus) => {
            let text = if bus.artifact_messages.is_empty() {
                String::new()
            } else {
                format!("{}\n", bus.artifact_messages.join("\n"))
            };
            collection.add("bus.txt", text.into_bytes());
            collection.summary.bus_messages = bus.summary_messages;
        }
        Err(error) => collection
            .assessment
            .unreadable
            .push(agent_core::diagnostic_assessment::ReadFailure {
                section: SummarySection::Bus,
                part: "bus messages".to_owned(),
                reason: crate::process::error_chain(&error),
            }),
    }
}

/// Records readable notifications or records an unreadable failure.
async fn observe_notifications(collection: &mut collect::Collection, home: &Home, team: &agent_core::state::Team) {
    let directory = match config::notification_directory(home).await {
        Ok(directory) => directory,
        Err(error) => {
            collection
                .assessment
                .unreadable
                .push(agent_core::diagnostic_assessment::ReadFailure {
                    section: SummarySection::Notifications,
                    part: "notifications configuration".to_owned(),
                    reason: crate::process::error_chain(&error),
                });
            return;
        }
    };
    match notifications::collect(&directory, team).await {
        Ok(notifs) => {
            let text = if notifs.is_empty() {
                String::new()
            } else {
                format!("{}\n", notifs.join("\n\n"))
            };
            collection.add("notifications.txt", text.into_bytes());
            collection.summary.notifications = notifs;
        }
        Err(error) => collection
            .assessment
            .unreadable
            .push(agent_core::diagnostic_assessment::ReadFailure {
                section: SummarySection::Notifications,
                part: "notifications".to_owned(),
                reason: crate::process::error_chain(&error),
            }),
    }
}

/// Records readable configuration and redacted secrets, or records an unreadable failure.
async fn observe_config(collection: &mut collect::Collection, home: &Home, secrets: &agent_core::config::Secrets) {
    let secret_names = config::format_secret_names(secrets);
    collection.add("secrets.toml", secret_names.into_bytes());

    let mut secret_keys = Vec::new();
    if secrets.github_token().is_some() {
        secret_keys.push("github_token".to_owned());
    }
    for key in secrets.agent_env().keys() {
        secret_keys.push(format!("agent_env.{key}"));
    }
    for key in secrets.notification_env().keys() {
        secret_keys.push(format!("notification_env.{key}"));
    }
    collection.summary.secret_keys = secret_keys;

    match config::collect_config(home).await {
        Ok(text) => {
            collection.add("agents.toml", text.clone().into_bytes());
            let redacted = collection.cut.redact_text(&text);
            collection.summary.configuration = Some(redacted);
        }
        Err(error) => {
            collection
                .assessment
                .unreadable
                .push(agent_core::diagnostic_assessment::ReadFailure {
                    section: SummarySection::DaemonAndVersions,
                    part: "configuration".to_owned(),
                    reason: crate::process::error_chain(&error),
                });
        }
    }
}

/// Records machine and resource observations, or records read failures.
async fn observe_machine(collection: &mut collect::Collection, home: &Home) {
    let procfs = std::path::Path::new("/proc");
    let temp_dir = std::env::temp_dir();
    let daemon_pid = collection.summary.daemon.as_ref().map(|d| d.pid);
    let obs = machine::observe(
        home,
        procfs,
        &temp_dir,
        daemon_pid,
        &collection.cut,
        collection.assessment.now,
    )
    .await;
    collection.summary.machine = obs.summary;
    collection.assessment.filesystems.extend(obs.filesystems);
    collection.assessment.unreadable.extend(obs.unreadable);
    collection.add("machine.txt", obs.artifact_bytes);
}

/// Machine presentation context captured by the CLI before it starts async workers.
#[derive(Debug, Clone)]
pub struct DebugLocation {
    /// Machine's current offset from UTC.
    pub local_offset: time::UtcOffset,
    /// Clone from which diagnostics were requested, when known.
    pub checkout: Option<PathBuf>,
}
