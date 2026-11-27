//! Optional findings selected and recorded before a model run starts.

use agent_core::config::{AbsolutePath, Config, InvalidValue, RepoName};
use agent_core::decide::findings::{Finding, InvalidRepoPath, RepoPath, item_paths, render_findings, select_findings};
use agent_core::ids::{InvalidBranchName, InvalidName, TaskId, TeamId};
use agent_core::local_time::Zone;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::state::{InvalidTimestamp, TaskRef, Timestamp};
use agent_runtime::agentbus::{AgentBus, AgentBusError};
use agent_runtime::clock::SharedClock;
use agent_runtime::findings::{ShownFindings, ShownFindingsError};
use agent_runtime::git::{ChangedFilesError, CommitIdentity, Git};
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::store::Home;
use std::path::PathBuf;
use tracing::Instrument as _;

/// Inputs shared by owner and pooled-worker prompt preparation.
pub(super) struct Delivery<'a> {
    /// Configuration including selection bounds.
    pub config: &'a Config,
    /// Task storage.
    pub home: &'a Home,
    /// Bus of this installation.
    pub bus: &'a AgentBus,
    /// Loaded prompt catalog.
    pub texts: &'a HarnessPrompts,
    /// Injected time source.
    pub clock: &'a SharedClock,
    /// Presentation time zone.
    pub zone: Zone,
}

/// Why an optional findings section could not be prepared.
#[derive(Debug, thiserror::Error)]
enum FindingsError {
    /// Bus query failed.
    #[error("cannot query open findings")]
    Bus(#[from] AgentBusError),
    /// Git could not identify the changed files.
    #[error("cannot identify run paths")]
    Git(#[from] ChangedFilesError),
    /// A configured path could not be used.
    #[error("invalid worktree path")]
    Path(#[from] InvalidValue),
    /// A run path could not be normalized.
    #[error("invalid run path")]
    RepoPath(#[from] InvalidRepoPath),
    /// An identity could not be used by git.
    #[error("invalid git identity")]
    Name(#[from] InvalidName),
    /// A base branch could not be used by git.
    #[error("invalid base branch")]
    Branch(#[from] InvalidBranchName),
    /// The injected clock is outside the supported range.
    #[error("cannot obtain findings selection time")]
    Time(#[from] InvalidTimestamp),
    /// Delivery bookkeeping failed.
    #[error("cannot record shown findings")]
    Shown(#[from] ShownFindingsError),
    /// A variable in the prompt catalog is invalid.
    #[error("invalid findings prompt variable")]
    Variable(#[from] VariablesError),
    /// The prompt catalog could not render the section.
    #[error("cannot render findings section")]
    Render(#[from] RenderTextError),
    /// Filesystem metadata or task-directory creation failed.
    #[error("cannot prepare findings path {path:?}")]
    Io {
        /// Path being prepared or inspected.
        path: PathBuf,
        /// Operating-system cause.
        source: std::io::Error,
    },
    /// A worktree path is not UTF-8.
    #[error("worktree path is not UTF-8: {path:?}")]
    NonUtf8 {
        /// The worktree.
        path: PathBuf,
    },
    /// An investigation's original task is missing from the daemon state.
    #[error("cannot find source task {task}")]
    MissingTask {
        /// Task whose branch must supply the run paths.
        task: TaskId,
    },
}

impl Delivery<'_> {
    /// Uses the step files rather than unrelated changes in the worker's checkout.
    pub(super) async fn for_request(
        &self,
        team: &TeamId,
        task: &TaskId,
        repo: &RepoName,
        request: agent_runtime::agentbus::RequestId,
    ) -> Option<String> {
        let result: Result<Option<String>, FindingsError> = async {
            let findings = self.bus.findings_in(repo, self.zone).await?;
            if findings.is_empty() {
                return Ok(None);
            }
            let work = self.bus.helper_work(request).await?;
            let paths = work
                .files
                .iter()
                .map(|path| RepoPath::parse(path.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            self.section(team, task, &findings, &paths).await
        }
        .await;
        match result {
            Ok(section) => section,
            Err(error) => {
                unavailable(team, task, &error);
                None
            }
        }
    }

    /// Selects from open findings, renders them and saves their IDs before returning the section.
    async fn section(
        &self,
        team: &TeamId,
        task: &TaskId,
        findings: &[Finding],
        paths: &[RepoPath],
    ) -> Result<Option<String>, FindingsError> {
        let directory = self.home.task_dir(task);
        let record = directory.join("findings-shown.json");
        let mut shown = ShownFindings::read(&record).await?;
        let limits = self.config.findings();
        let selected = select_findings(
            findings,
            paths,
            shown.ids(),
            task,
            Timestamp::now(self.clock)?,
            limits.keep_days(),
            usize::from(limits.max_shown()),
            usize::from(limits.max_chars()),
        );
        if selected.is_empty() {
            return Ok(None);
        }
        let values = Variables::new().with("findings", render_findings(&selected, self.zone))?;
        let section = self.texts.render(HarnessPrompt::Findings, &values)?;
        let ids = selected.iter().map(|row| row.finding.id).collect::<Vec<_>>();
        let scores = selected.iter().map(|row| row.score).collect::<Vec<_>>();
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|source| FindingsError::Io {
                path: directory,
                source,
            })?;
        shown.save(&record, ids.iter().copied()).await?;
        tracing::info!(event = LogEvent::FindingsShown.as_str(), team = %team, task = %task, ids = ?ids.iter().map(ToString::to_string).collect::<Vec<_>>(), scores = ?scores, "findings selected for run");
        Ok(Some(section))
    }
}

/// Loads the task's paths and applies the same delivery policy as pooled workers.
pub(super) async fn for_owner(ctx: &super::start::Context<'_>, found: &TaskRef<'_>, zone: Zone) -> Option<String> {
    let config = &ctx.settings.config;
    let bus = AgentBus::new(config.binaries().agentbus.clone(), ctx.home.absolute_root().clone());
    let delivery = Delivery {
        config,
        home: ctx.home,
        bus: &bus,
        texts: ctx.texts,
        clock: ctx.clock,
        zone,
    };
    let result: Result<Option<String>, FindingsError> = async {
        let findings = bus.findings_in(found.team.repo(), zone).await?;
        if findings.is_empty() {
            return Ok(None);
        }
        let paths = owner_paths(ctx, found, &bus).await?;
        delivery
            .section(found.team.id(), found.member.task(), &findings, &paths)
            .await
    }
    .instrument(tracing::info_span!("findings", team = %found.team.id(), task = %found.member.task()))
    .await;
    match result {
        Ok(section) => section,
        Err(error) => {
            unavailable(found.team.id(), found.member.task(), &error);
            None
        }
    }
}

/// The investigator uses its requester's branch; other owner runs use their own branch.
async fn owner_paths(
    ctx: &super::start::Context<'_>,
    found: &TaskRef<'_>,
    bus: &AgentBus,
) -> Result<Vec<RepoPath>, FindingsError> {
    if let Some(crate::daemon::team::Why::Delegated(request)) = ctx.why {
        let work = bus.helper_work(request.id).await?;
        return work
            .files
            .iter()
            .map(|path| RepoPath::parse(path.as_str()))
            .collect::<Result<_, _>>()
            .map_err(FindingsError::from);
    }
    let source = if let Some(record) = found
        .team
        .investigations()
        .iter()
        .find(|record| &record.task == found.member.task())
    {
        let request = bus.inv_get(record.id).await?;
        ctx.state
            .find_task(&request.from_task)
            .ok_or(FindingsError::MissingTask {
                task: request.from_task,
            })?
    } else {
        TaskRef {
            team: found.team,
            role: found.role,
            member: found.member,
        }
    };
    let worktree = ctx.home.worktree_dir(source.team.repo(), source.member.task());
    let absolute = worktree
        .to_str()
        .ok_or_else(|| FindingsError::NonUtf8 { path: worktree.clone() })?
        .parse::<AbsolutePath>()?;
    let identity = CommitIdentity::for_agent(ctx.settings.config.git(), &source.member.agent().name().parse()?);
    let mut paths = Git::new(absolute, identity).changed_files(&"main".parse()?).await?;
    if let Some(item) = super::start::assigned_item(source.team, source.member.agent()) {
        for text in [
            Some(item.title.as_str()),
            Some(item.description.as_str()),
            item.expected_result.as_deref(),
            item.checks.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            for path in item_paths(text) {
                let full = worktree.join(path.as_str());
                match tokio::fs::metadata(&full).await {
                    Ok(metadata) if metadata.is_file() || metadata.is_dir() => paths.push(path),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(FindingsError::Io { path: full, source }),
                }
            }
        }
    }
    Ok(paths)
}

/// Records the optional fallback with task and team scope, once where it is handled.
fn unavailable(team: &TeamId, task: &TaskId, error: &FindingsError) {
    tracing::warn!(event = LogEvent::FindingsUnavailable.as_str(), team = %team, task = %task, error = %agent_runtime::process::error_chain(error), "run starts without findings");
}
