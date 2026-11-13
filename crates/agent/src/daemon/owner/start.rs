//! Making a queued task ready to run: what its run needs from the state and from the configuration.

use std::collections::BTreeMap;
use std::io;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

use agent_core::cli::{McpServerKind, RunMode, SessionId};
use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, InvalidValue, ModelName};
use agent_core::ids::{BranchName, GenerateIdError, InvalidBranchName, InvalidName, RunId, TaskId};
use agent_core::kinds::{CliKind, Tier};
use agent_core::state::{ItemPhase, Member, Participant, Question, RunEnd, RunState, State, TaskRef, Team};
use agent_core::status::RunOutcome;
use agent_core::task_list::Item;
use agent_runtime::agentbus::{AgentBus, BusTier};
use agent_runtime::clock::SharedClock;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::prompts::PromptLoader;
use agent_runtime::questions_prompts::QuestionsPrompts;
use agent_runtime::run::RunJob;
use agent_runtime::run::compose::{ComposeError, PromptInput, PromptKind, PromptTexts, ResumeReason, compose};
use agent_runtime::settings::Settings;
use agent_runtime::store::Home;
use agent_runtime::team_prompts::TeamPrompts;
use rand::{Rng, RngExt};

use crate::daemon::questions::{self, QuestionsProblem, TextError};
use crate::daemon::team::{TeamPromptError, TeamRun, Why, agent_name, shape_with_answers, task_branch};

/// The value of the `--role` of `agentbus serve` for a run of an investigator.
const BUS_ROLE_INVESTIGATOR: &str = "investigator";

/// The value of the `--role` of `agentbus serve` for every other run.
const BUS_ROLE_WORKER: &str = "worker";

/// Why a run cannot be made ready.
#[derive(Debug, thiserror::Error)]
pub(super) enum StartProblem {
    /// The task is not in the state: it was there a moment ago, so this is a mistake of the daemon.
    #[error("task {task} is not in the state")]
    NoSuchTask {
        /// The task.
        task: TaskId,
    },
    /// The agent of the task is not in the configuration.
    #[error("the agent {name} is not in the configuration (agents.toml): add it back, or stop the task")]
    AgentGone {
        /// The agent's name.
        name: String,
    },
    /// The worktree of the task is not there.
    #[error("the worktree {path:?} of the task cannot be used")]
    NoWorktree {
        /// The worktree.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The worktree is not a directory.
    #[error("the worktree {path:?} of the task is not a directory")]
    WorktreeNotDirectory {
        /// The worktree.
        path: PathBuf,
    },
    /// A path is not valid text or not an absolute path.
    #[error("the path {path:?} cannot be used")]
    Path {
        /// The path.
        path: PathBuf,
        /// What is wrong with it, if the text was refused.
        source: Option<InvalidValue>,
    },
    /// The question rounds of the task are used up.
    #[error("the task has no round of questions left")]
    RoundsExhausted,
    /// The prompt cannot be composed.
    #[error("cannot compose the prompt")]
    Prompt(#[from] ComposeError),
    /// An ID cannot be made.
    #[error("cannot make the ID of the run")]
    Id(#[from] GenerateIdError),
    /// The repository of the task is not in the configuration any more.
    #[error("the repository {repo} is not in the configuration (agents.toml): add it back, or stop the task")]
    RepoGone {
        /// The repository.
        repo: String,
    },
    /// The questions file the human answered cannot be read.
    #[error("cannot read the answers of the human")]
    Answers(#[from] QuestionsProblem),
    /// What the task is told about its answers cannot be made.
    #[error("cannot make what the task is told about its answers")]
    AnswersText(#[from] TextError),
    /// What the team tells the run cannot be made.
    #[error("cannot make what the team tells the run")]
    Team(#[from] TeamPromptError),
    /// The name of an agent is not an agent name.
    #[error("the name of an agent is not an agent name")]
    Name(#[from] InvalidName),
    /// The branch of the task is not a branch name.
    #[error("the branch of the task is not a branch name")]
    Branch(#[from] InvalidBranchName),
    /// A session ID is not valid.
    #[error("a session ID is not valid")]
    Session(#[from] InvalidValue),
}

/// What a run is made ready from.
pub(super) struct Context<'a> {
    /// The state.
    pub(super) state: &'a State,
    /// The configuration and the secrets.
    pub(super) settings: &'a Settings,
    /// The run prompts.
    pub(super) texts: &'a HarnessPrompts,
    /// The texts of the teams.
    pub(super) team_texts: &'a TeamPrompts,
    /// The texts of the questions and their answers.
    pub(super) question_texts: &'a QuestionsPrompts,
    /// The rules.
    pub(super) rules: &'a PromptLoader,
    /// The home.
    pub(super) home: &'a Home,
    /// The clock the IDs are made from.
    pub(super) clock: &'a SharedClock,
    /// Why the task of a team starts a run now, if it is more than the plain continuation of the task.
    pub(super) why: Option<&'a Why>,
}

/// A run that is ready to start.
pub(super) struct Plan {
    /// The ID of the run.
    pub(super) run: RunId,
    /// The session to record when the run begins.
    pub(super) recorded_session: Option<agent_core::state::SessionId>,
    /// What the engine runs.
    pub(super) job: RunJob,
}

/// How the CLI of an agent is run.
struct AgentSetup {
    /// The CLI.
    cli: CliKind,
    /// The model.
    model: Option<ModelName>,
    /// The extra arguments.
    args: CliArgs,
}

/// The CLI, model and arguments the configuration gives `participant`.
fn agent_setup(config: &Config, participant: &Participant) -> Option<AgentSetup> {
    match participant {
        Participant::Normal(name) | Participant::Worker(name) => config.agent(name).map(|agent| AgentSetup {
            cli: agent.cli,
            model: agent.model.clone(),
            args: agent.args.clone(),
        }),
        Participant::Investigator(name) => config.investigator(name).map(|agent| AgentSetup {
            cli: agent.cli,
            model: agent.model.clone(),
            args: agent.args.clone(),
        }),
    }
}

/// Makes the run of `task` ready.
///
/// # Errors
/// [`StartProblem`].
pub(super) async fn plan(ctx: &Context<'_>, rng: &mut impl Rng, task: &TaskId) -> Result<Plan, StartProblem> {
    let found = ctx
        .state
        .find_task(task)
        .ok_or_else(|| StartProblem::NoSuchTask { task: task.clone() })?;
    let (team, role, member) = (found.team, found.role, found.member);
    let participant = member.agent();
    let config = &ctx.settings.config;
    let setup = agent_setup(config, participant).ok_or_else(|| StartProblem::AgentGone {
        name: participant.name().to_owned(),
    })?;
    let working = ctx.home.worktree_dir(team.repo(), task);
    check_directory(&working).await?;
    let working_dir = absolute(&working)?;
    let round = next_round(member).ok_or(StartProblem::RoundsExhausted)?;
    let questions_file = ctx.home.questions_file(task, round);
    let session = member
        .last_session()
        .map(|session| SessionId::from_str(session.as_str()))
        .transpose()?;
    let kind = prompt_kind(member, session.as_ref(), config);
    let item = assigned_item(team, participant);
    let input = PromptInput {
        kind,
        agent: participant.name(),
        tier: participant.tier(),
        role,
        task,
        repo: team.repo(),
        worktree: &working,
        questions_file: &questions_file,
        round,
        brief: team.task_list().list().brief(),
        item,
        // Checkpoints live on the agentbus, which a team's life (the next item) creates: none yet.
        checkpoint: None,
    };
    let texts = PromptTexts {
        harness: ctx.texts,
        rules: ctx.rules,
    };
    let prompt = compose(&texts, &input)?;
    let answers = answers_text(ctx, member).await?;
    let prompt = team_prompt(ctx, &texts, &found, prompt, answers.as_deref())?;
    let run = RunId::generate(ctx.clock, rng)?;
    let (mode, recorded_session) = if let Some(session_id) = session {
        (RunMode::Resume { session_id }, member.last_session().cloned())
    } else {
        let session_id = new_session_id(rng)?;
        // Only Claude Code lets the caller name the session; the others report their own when they end.
        let recorded = if setup.cli == CliKind::Claude {
            Some(agent_core::state::SessionId::from_str(session_id.as_str())?)
        } else {
            None
        };
        (RunMode::New { session_id }, recorded)
    };
    let mcp_servers = mcp_servers(ctx, team, participant, task, &working, &work_branch(team, member)?)?;
    Ok(Plan {
        run: run.clone(),
        recorded_session,
        job: RunJob {
            task: task.clone(),
            run,
            cli: setup.cli,
            model: setup.model,
            extra_args: setup.args,
            mode,
            prompt,
            working_dir,
            mcp_servers,
        },
    })
}

/// The round of questions the human answered and no run has read since: the last round is answered, and the latest run
/// that read its prompt ended asking. A run that did not read it (it crashed, was stopped, hung, hit a limit) leaves the
/// answers to the next one. It is worked out from the state, so that it holds after a restart.
fn answered_round(member: &Member) -> Option<&Question> {
    let question = member.questions().last().filter(|question| !question.is_open())?;
    for run in member.runs().iter().rev() {
        match run.state() {
            RunState::Ended {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Questions,
                },
                ..
            } => return Some(question),
            // The agent said something to the end: it had the answers, if it was given them.
            RunState::Ended {
                end: RunEnd::Reported { .. } | RunEnd::NoStatus { exit_code: Some(0) },
                ..
            } => return None,
            RunState::Ended {
                end:
                    RunEnd::NoStatus {
                        exit_code: None | Some(_),
                    }
                    | RunEnd::QuotaExhausted { .. }
                    | RunEnd::RateLimited
                    | RunEnd::Hung
                    | RunEnd::Stopped,
                ..
            }
            | RunState::Running => {}
        }
    }
    None
}

/// What the human answered to the round the last run asked, the way the run is told; `None` when there is no such
/// round.
async fn answers_text(ctx: &Context<'_>, member: &Member) -> Result<Option<String>, StartProblem> {
    let Some(question) = answered_round(member) else {
        return Ok(None);
    };
    let loaded = questions::read(question.file().as_path()).await?;
    let answers = questions::answers_prompt(&loaded.file, question.round(), ctx.question_texts)?;
    Ok(Some(answers))
}

/// The prompt of a run that is shaped by the team the task is in, and told the `answers` of the human; a task without a
/// team keeps `composed`, with the answers.
fn team_prompt(
    ctx: &Context<'_>,
    texts: &PromptTexts<'_>,
    found: &TaskRef<'_>,
    composed: String,
    answers: Option<&str>,
) -> Result<String, StartProblem> {
    let (team, member) = (found.team, found.member);
    let base = ctx
        .settings
        .config
        .repo(team.repo())
        .map(|repo| repo.base_branch.clone())
        .ok_or_else(|| StartProblem::RepoGone {
            repo: team.repo().to_string(),
        })?;
    let branch = work_branch(team, member)?;
    Ok(shape_with_answers(
        ctx.team_texts,
        texts,
        &TeamRun {
            team,
            role: found.role,
            branch: &branch,
            base: &base,
            why: ctx.why,
            new_session: member.last_session().is_none(),
        },
        composed,
        answers,
    )?)
}

/// Checks that `path` is a directory.
async fn check_directory(path: &std::path::Path) -> Result<(), StartProblem> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(StartProblem::WorktreeNotDirectory {
            path: path.to_path_buf(),
        }),
        Err(source) => Err(StartProblem::NoWorktree {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// `path` as an absolute path.
fn absolute(path: &std::path::Path) -> Result<AbsolutePath, StartProblem> {
    let text = path.to_str().ok_or_else(|| StartProblem::Path {
        path: path.to_path_buf(),
        source: None,
    })?;
    AbsolutePath::from_str(text).map_err(|source| StartProblem::Path {
        path: path.to_path_buf(),
        source: Some(source),
    })
}

/// The number of the round of questions the task would ask next.
pub(super) fn next_round(member: &Member) -> Option<NonZeroU32> {
    u32::try_from(member.questions().len())
        .ok()
        .and_then(|asked| asked.checked_add(1))
        .and_then(NonZeroU32::new)
}

/// What the prompt of the next run tells: the whole task, or what interrupted the session it resumes.
fn prompt_kind(member: &Member, session: Option<&SessionId>, config: &Config) -> PromptKind {
    if session.is_none() {
        return PromptKind::First;
    }
    let last_end = member.runs().last().and_then(|run| match run.state() {
        RunState::Ended { end, .. } => Some(end),
        RunState::Running => None,
    });
    match last_end {
        Some(RunEnd::NoStatus { exit_code: None }) => PromptKind::Resume(ResumeReason::Restart),
        Some(RunEnd::Hung) => PromptKind::Resume(ResumeReason::Hang {
            after: config.watchdog().hang_minutes,
        }),
        Some(RunEnd::QuotaExhausted { .. } | RunEnd::RateLimited) => PromptKind::Resume(ResumeReason::Limit),
        // Any other end (a failure the human lets the agent retry, a stop) gets the whole task again.
        Some(RunEnd::Reported { .. } | RunEnd::NoStatus { exit_code: Some(_) } | RunEnd::Stopped) | None => {
            PromptKind::First
        }
    }
}

/// The item of the team that `participant` is working on, if it has one.
fn assigned_item<'a>(team: &'a Team, participant: &Participant) -> Option<&'a Item> {
    team.items().iter().find_map(|progress| match progress.phase() {
        ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. }
            if assignee.as_str() == participant.name() =>
        {
            team.task_list().list().item(progress.id())
        }
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => None,
    })
}

/// The branch `member` works on: the branch of its item, or the branch its task started on.
fn work_branch(team: &Team, member: &Member) -> Result<BranchName, StartProblem> {
    let item_branch = team.items().iter().find_map(|progress| match progress.phase() {
        ItemPhase::InProgress { assignee, branch } | ItemPhase::Fixing { assignee, branch, .. }
            if assignee.as_str() == member.agent().name() =>
        {
            Some(branch.clone())
        }
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => None,
    });
    match item_branch {
        Some(branch) => Ok(branch),
        None => Ok(task_branch(&agent_name(member.agent())?, member.task())?),
    }
}

/// A session ID in the form of a version 4 UUID, which Claude Code accepts for `--session-id`.
fn new_session_id(rng: &mut impl Rng) -> Result<SessionId, InvalidValue> {
    let mut bytes: [u8; 16] = rng.random();
    // The version (4) and the variant (RFC 4122) of a UUID.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    let mut text = String::with_capacity(UUID_LEN);
    for (index, digit) in hex.chars().enumerate() {
        if UUID_DASHES.contains(&index) {
            text.push('-');
        }
        text.push(digit);
    }
    SessionId::from_str(&text)
}

/// The length of a UUID in text.
const UUID_LEN: usize = 36;

/// The positions of the hex digits of a UUID before which a dash stands.
const UUID_DASHES: [usize; 4] = [8, 12, 16, 20];

/// The MCP servers of the run of `task`: the agentbus, and Serena when the configuration has it.
fn mcp_servers(
    ctx: &Context<'_>,
    team: &Team,
    participant: &Participant,
    task: &TaskId,
    working: &std::path::Path,
    branch: &BranchName,
) -> Result<BTreeMap<McpServerKind, CommandLine>, StartProblem> {
    let binaries = ctx.settings.config.binaries();
    let home = absolute(ctx.home.root())?;
    let bus_dir = AgentBus::new(binaries.agentbus.clone(), home).bus_dir();
    let (tier, role) = match participant.tier() {
        Tier::Normal => (BusTier::Normal, BUS_ROLE_WORKER),
        Tier::Worker => (BusTier::Worker, BUS_ROLE_WORKER),
        Tier::Investigator => (BusTier::Normal, BUS_ROLE_INVESTIGATOR),
    };
    let text = |path: &std::path::Path| {
        path.to_str().map(str::to_owned).ok_or_else(|| StartProblem::Path {
            path: path.to_path_buf(),
            source: None,
        })
    };
    let words = vec![
        text(binaries.agentbus.as_path())?,
        "serve".to_owned(),
        "--agent".to_owned(),
        participant.name().to_owned(),
        "--task".to_owned(),
        task.to_string(),
        "--task-dir".to_owned(),
        text(&ctx.home.task_dir(task))?,
        "--repo".to_owned(),
        team.repo().to_string(),
        "--worktree".to_owned(),
        text(working)?,
        "--branch".to_owned(),
        branch.to_string(),
        "--team".to_owned(),
        team.id().to_string(),
        "--tier".to_owned(),
        tier.to_string(),
        "--role".to_owned(),
        role.to_owned(),
        "--home".to_owned(),
        text(ctx.home.root())?,
        "--dir".to_owned(),
        text(&bus_dir)?,
    ];
    let mut servers = BTreeMap::new();
    servers.insert(McpServerKind::Agentbus, CommandLine::try_from(words)?);
    if let Some(serena) = &binaries.serena {
        let words = vec![
            text(serena.as_path())?,
            "start-mcp-server".to_owned(),
            "--project".to_owned(),
            text(working)?,
            "--open-web-dashboard".to_owned(),
            "false".to_owned(),
        ];
        servers.insert(McpServerKind::Serena, CommandLine::try_from(words)?);
    }
    Ok(servers)
}

#[cfg(test)]
mod tests;
