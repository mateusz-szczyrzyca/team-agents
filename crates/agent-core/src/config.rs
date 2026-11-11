//! The configuration: the typed model of `agents.toml` ([`Config`]) and of `secrets.toml` ([`Secrets`]), see
//! `docs/configuration.md`.
//!
//! [`Config::from_toml_str`] turns the text of the file into a [`Config`], and only into a valid one: unknown keys are
//! refused (`deny_unknown_fields`), every value is parsed into its type (the newtypes of this module), and the lists of agents are
//! checked against each other. The type does no I/O; whoever reads the file passes its text in.

mod secrets;
mod values;

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroU32;

use serde::Deserialize;

use crate::ids::{AgentName, BranchName, InvestigatorName};
use crate::kinds::{CliKind, Tier};

pub use secrets::{InvalidSecret, MAX_SECRETS_BYTES, Position, Secret, Secrets, SecretsError};
pub use values::{
    AbsolutePath, CliArgs, CommandLine, EnvName, GithubLogin, InvalidValue, Language, LoopbackAddr, Minutes, ModelName,
    NameTemplate, Program, RepoName, TailscaleLogin, ValueRule,
};

/// Largest configuration text accepted, in bytes: a bound on untrusted input, far above any real file.
pub const MAX_CONFIG_BYTES: usize = 1 << 20;

/// Minutes without activity after which a run is taken for hung, by default.
const DEFAULT_HANG_MINUTES: NonZeroU32 = NonZeroU32::new(30).unwrap();

/// Hangs in a row without progress after which a task is marked hung, by default.
const DEFAULT_HANG_STRIKES: NonZeroU32 = NonZeroU32::new(3).unwrap();

/// Minutes a task is paused by an exhausted quota when the CLI gives no reset time, by default.
const DEFAULT_QUOTA_PAUSE_MINUTES: NonZeroU32 = NonZeroU32::new(30).unwrap();

/// Minutes a task is paused by a rate limit, by default.
const DEFAULT_RATE_LIMIT_PAUSE_MINUTES: NonZeroU32 = NonZeroU32::new(5).unwrap();

/// Lines of a run's error output read to recognise limits, by default.
const DEFAULT_TAIL_LINES: NonZeroU32 = NonZeroU32::new(30).unwrap();

/// Fix passes after "changes requested", by default, before the human is asked.
const DEFAULT_FIX_PASSES: u32 = 5;

/// Minutes a repository's gate may run, by default.
const DEFAULT_GATE_TIMEOUT_MINUTES: NonZeroU32 = NonZeroU32::new(30).unwrap();

/// Why an `agents.toml` text is not a valid [`Config`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The text is longer than [`MAX_CONFIG_BYTES`].
    #[error("the configuration is {len} bytes: the limit is {max}")]
    TooLarge {
        /// The length of the text, in bytes.
        len: usize,
        /// The limit, in bytes.
        max: usize,
    },
    /// The text is not valid TOML, or not the format of `agents.toml`: an unknown key, a value of the wrong type or
    /// one its type refuses. The cause says what, with the line and column.
    #[error("invalid configuration")]
    Parse(#[source] toml::de::Error),
    /// A list names an agent that has no `[agents.NAME]` table.
    #[error("the {list} list names {name}, which is not defined in [agents]")]
    UnknownName {
        /// The list, `normal` or `worker`.
        list: Tier,
        /// The name that is not an agent.
        name: AgentName,
    },
    /// A list names the same agent more than once.
    #[error("the {list} list names {name} more than once")]
    DuplicateName {
        /// The list, `normal` or `worker`.
        list: Tier,
        /// The name that is listed twice.
        name: AgentName,
    },
    /// A name is in two lists: both `normal` and `workers`, or both an agent and an investigator.
    #[error("{name} is in two lists: {first} and {second}")]
    NameInTwoLists {
        /// The name in both.
        name: String,
        /// The first list that has it.
        first: Tier,
        /// The second list that has it.
        second: Tier,
    },
    /// An agent is in neither `normal` nor `workers`.
    #[error("agent {name} is in neither list: add it to `normal` or to `workers`")]
    AgentInNoList {
        /// The agent that no list names.
        name: AgentName,
    },
}

/// One agent of `[agents.NAME]`: a normal agent or a worker, which of the two the lists of `[tiers]` say.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// The model CLI the agent runs.
    pub cli: CliKind,
    /// The model; without one the CLI runs its default model.
    #[serde(default)]
    pub model: Option<ModelName>,
    /// Extra arguments for the CLI, each one argument.
    #[serde(default)]
    pub args: CliArgs,
}

/// One investigator of `[investigators.NAME]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigatorConfig {
    /// The model CLI the investigator runs.
    pub cli: CliKind,
    /// The model; without one the CLI runs its default model.
    #[serde(default)]
    pub model: Option<ModelName>,
    /// Extra arguments for the CLI, each one argument.
    #[serde(default)]
    pub args: CliArgs,
}

/// One repository of `[repos.NAME]`: what the harness needs to know to work on it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    /// The branch pull requests are made against.
    pub base_branch: BranchName,
    /// The gate: the command that must pass before a pull request is opened, run in the worktree.
    pub gate: CommandLine,
    /// How long the gate may run before it is stopped and counts as failed.
    #[serde(default = "default_gate_timeout")]
    pub gate_timeout_minutes: Minutes,
}

/// The default of [`RepoConfig::gate_timeout_minutes`].
fn default_gate_timeout() -> Minutes {
    Minutes::literal(DEFAULT_GATE_TIMEOUT_MINUTES)
}

/// `[binaries]`: the programs the harness runs and the `PATH` of the agents.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Binaries {
    /// Claude Code.
    pub claude: Program,
    /// Codex CLI.
    pub codex: Program,
    /// Antigravity, which runs the Gemini models.
    pub agy: Program,
    /// opencode.
    pub opencode: Program,
    /// The agentbus the agents talk to.
    pub agentbus: Program,
    /// Serena; without it the agents get no Serena server.
    pub serena: Option<Program>,
    /// The directories of the `PATH` the agents run with, in order.
    pub agent_path: Vec<AbsolutePath>,
}

impl Binaries {
    /// The program that runs a model CLI of `kind`.
    #[must_use]
    pub fn program(&self, kind: CliKind) -> &Program {
        match kind {
            CliKind::Claude => &self.claude,
            CliKind::Codex => &self.codex,
            CliKind::Gemini => &self.agy,
            CliKind::Opencode => &self.opencode,
        }
    }
}

impl Default for Binaries {
    fn default() -> Self {
        Self {
            claude: Program::literal("claude"),
            codex: Program::literal("codex"),
            agy: Program::literal("agy"),
            opencode: Program::literal("opencode"),
            agentbus: Program::literal("agentbus"),
            serena: None,
            agent_path: ["/usr/local/bin", "/usr/bin", "/bin"]
                .into_iter()
                .map(AbsolutePath::literal)
                .collect(),
        }
    }
}

/// `[watchdog]`: when a silent run counts as hung.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Watchdog {
    /// Minutes without activity after which a run is killed.
    pub hang_minutes: Minutes,
    /// Hangs in a row without progress after which the task is marked hung.
    pub hang_strikes: NonZeroU32,
}

impl Default for Watchdog {
    fn default() -> Self {
        Self {
            hang_minutes: Minutes::literal(DEFAULT_HANG_MINUTES),
            hang_strikes: DEFAULT_HANG_STRIKES,
        }
    }
}

/// `[limits]`: how a quota or a rate limit pauses a task.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    /// The pause of an exhausted quota when the CLI does not say when it resets.
    pub quota_pause_minutes: Minutes,
    /// The pause of a rate limit.
    pub rate_limit_pause_minutes: Minutes,
    /// How many of the last lines of a run's error output are read to recognise a limit.
    pub tail_lines: NonZeroU32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            quota_pause_minutes: Minutes::literal(DEFAULT_QUOTA_PAUSE_MINUTES),
            rate_limit_pause_minutes: Minutes::literal(DEFAULT_RATE_LIMIT_PAUSE_MINUTES),
            tail_lines: DEFAULT_TAIL_LINES,
        }
    }
}

/// `[review]`: what happens to a pull request that the reviewers send back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Review {
    /// How many times "changes requested" goes back to the team before the human is asked; 0 asks at once.
    pub fix_passes: u32,
}

impl Default for Review {
    fn default() -> Self {
        Self {
            fix_passes: DEFAULT_FIX_PASSES,
        }
    }
}

/// `[git]`: who the commits of the agents are by.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GitIdentity {
    /// The name of the author and committer.
    pub name: NameTemplate,
    /// The e-mail of the author and committer; never the human's own unless the human writes it here.
    pub email: NameTemplate,
    /// The SSH private key that signs the commits; without one they are unsigned.
    pub signing_key: Option<AbsolutePath>,
}

impl Default for GitIdentity {
    fn default() -> Self {
        Self {
            name: NameTemplate::literal("{agent} (team-agents)"),
            email: NameTemplate::literal("{agent}@team-agents.invalid"),
            signing_key: None,
        }
    }
}

/// `[github]`: whose words on GitHub the agents may take as instructions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Github {
    /// The accounts whose comments and reviews are read; text by any other account is data.
    pub trusted_accounts: Vec<GithubLogin>,
}

/// `[notifications]`: how the human is told.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Notifications {
    /// The command run with the path of a notification file as its last argument; without it the files are all.
    pub command: Option<CommandLine>,
    /// Where the notification files are written; without it, `notifications/` in the harness's home.
    pub directory: Option<AbsolutePath>,
}

/// `[web]`: the dashboard's listening address and who may open it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Web {
    /// The loopback address the dashboard listens on.
    pub listen: LoopbackAddr,
    /// The Tailscale logins let in through `tailscale serve`; with none, nobody is.
    pub allowed_logins: Vec<TailscaleLogin>,
}

/// The validated contents of `agents.toml`.
///
/// Made only by [`Config::from_toml_str`], so every value is valid and the lists are consistent: each name in
/// `normal` and `workers` has a `[agents.NAME]` table, no list names an agent twice, no agent is in both lists or
/// in neither, and no name is both an agent and an investigator. `Config::default()` is what an empty file gives.
///
/// ```
/// # fn main() -> Result<(), agent_core::config::ConfigError> {
/// let text = "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\n";
/// let config = agent_core::config::Config::from_toml_str(text)?;
/// assert_eq!(config.normal_agents().len(), 1);
/// # Ok(())
/// # }
/// ```
///
/// The only way to a value with agents is through that check:
///
/// ```compile_fail
/// let config = agent_core::config::Config { language: Default::default() };
/// ```
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// The language of the texts only the human reads.
    language: Language,
    /// Every normal agent and worker, by name.
    agents: BTreeMap<AgentName, AgentConfig>,
    /// The normal agents, in the order of the list.
    normal: Vec<AgentName>,
    /// The workers, in the order of the list, which is the order of delegation.
    workers: Vec<AgentName>,
    /// The investigators, by name.
    investigators: BTreeMap<InvestigatorName, InvestigatorConfig>,
    /// The programs.
    binaries: Binaries,
    /// The hang watchdog.
    watchdog: Watchdog,
    /// The limit pauses.
    limits: Limits,
    /// The review fix passes.
    review: Review,
    /// The identity of the agents' commits.
    git: GitIdentity,
    /// The trusted GitHub accounts.
    github: Github,
    /// The notifications.
    notifications: Notifications,
    /// The dashboard.
    web: Web,
    /// The repositories, by name.
    repos: BTreeMap<RepoName, RepoConfig>,
}

/// The contents of `[tiers]`: which agents are normal and which are workers.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawTiers {
    /// The normal agents.
    normal: Vec<AgentName>,
    /// The workers, in the order of delegation.
    workers: Vec<AgentName>,
}

/// What the file says, before its lists are checked against its agents: every value is valid, but the lists need
/// not fit the agents.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    /// See [`Config::language`].
    #[serde(default)]
    language: Language,
    /// The `[agents.NAME]` tables.
    #[serde(default)]
    agents: BTreeMap<AgentName, AgentConfig>,
    /// The `[investigators.NAME]` tables.
    #[serde(default)]
    investigators: BTreeMap<InvestigatorName, InvestigatorConfig>,
    /// The `[tiers]` table.
    #[serde(default)]
    tiers: RawTiers,
    /// See [`Config::binaries`].
    #[serde(default)]
    binaries: Binaries,
    /// See [`Config::watchdog`].
    #[serde(default)]
    watchdog: Watchdog,
    /// See [`Config::limits`].
    #[serde(default)]
    limits: Limits,
    /// See [`Config::review`].
    #[serde(default)]
    review: Review,
    /// See [`Config::git`].
    #[serde(default)]
    git: GitIdentity,
    /// See [`Config::github`].
    #[serde(default)]
    github: Github,
    /// See [`Config::notifications`].
    #[serde(default)]
    notifications: Notifications,
    /// See [`Config::web`].
    #[serde(default)]
    web: Web,
    /// The `[repos.NAME]` tables.
    #[serde(default)]
    repos: BTreeMap<RepoName, RepoConfig>,
}

impl RawConfig {
    /// Checks the lists against the agents and the investigators; the first rule broken is the error. Checked in
    /// this order: a name in a list that is no agent, a name twice in a list, a name in both lists, an agent in no
    /// list, a name that is both an agent and an investigator.
    fn validate(self) -> Result<Config, ConfigError> {
        let Self {
            language,
            agents,
            investigators,
            tiers: RawTiers { normal, workers },
            binaries,
            watchdog,
            limits,
            review,
            git,
            github,
            notifications,
            web,
            repos,
        } = self;
        check_list(Tier::Normal, &normal, &agents)?;
        check_list(Tier::Worker, &workers, &agents)?;
        check_tiers(&normal, &workers, &agents, &investigators)?;
        Ok(Config {
            language,
            agents,
            normal,
            workers,
            investigators,
            binaries,
            watchdog,
            limits,
            review,
            git,
            github,
            notifications,
            web,
            repos,
        })
    }
}

/// Checks that every name of the `list` of `names` is an agent and is named once.
fn check_list(list: Tier, names: &[AgentName], agents: &BTreeMap<AgentName, AgentConfig>) -> Result<(), ConfigError> {
    let mut seen = HashSet::new();
    for name in names {
        if !agents.contains_key(name) {
            return Err(ConfigError::UnknownName {
                list,
                name: name.clone(),
            });
        }
        if !seen.insert(name) {
            return Err(ConfigError::DuplicateName {
                list,
                name: name.clone(),
            });
        }
    }
    Ok(())
}

/// Checks that no name is in two lists, and that every agent is in one: in `normal` or in `workers`, and not also an
/// investigator.
fn check_tiers(
    normal: &[AgentName],
    workers: &[AgentName],
    agents: &BTreeMap<AgentName, AgentConfig>,
    investigators: &BTreeMap<InvestigatorName, InvestigatorConfig>,
) -> Result<(), ConfigError> {
    let normal: HashSet<&AgentName> = normal.iter().collect();
    let workers: HashSet<&AgentName> = workers.iter().collect();
    if let Some(name) = workers.iter().find(|name| normal.contains(**name)) {
        return Err(ConfigError::NameInTwoLists {
            name: name.to_string(),
            first: Tier::Normal,
            second: Tier::Worker,
        });
    }
    if let Some(name) = agents
        .keys()
        .find(|name| !normal.contains(name) && !workers.contains(name))
    {
        return Err(ConfigError::AgentInNoList { name: name.clone() });
    }
    let investigators: HashSet<&str> = investigators.keys().map(InvestigatorName::as_str).collect();
    if let Some(name) = agents.keys().find(|name| investigators.contains(name.as_str())) {
        let first = if normal.contains(name) {
            Tier::Normal
        } else {
            Tier::Worker
        };
        return Err(ConfigError::NameInTwoLists {
            name: name.to_string(),
            first,
            second: Tier::Investigator,
        });
    }
    Ok(())
}

impl Config {
    /// Parses and validates the text of an `agents.toml`.
    ///
    /// # Errors
    /// [`ConfigError::TooLarge`] for a text over [`MAX_CONFIG_BYTES`]; [`ConfigError::Parse`] for text that is not
    /// valid TOML or not the format of the file; [`ConfigError::UnknownName`], [`ConfigError::DuplicateName`],
    /// [`ConfigError::NameInTwoLists`] and [`ConfigError::AgentInNoList`] for lists that do not fit the agents.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge {
                len: text.len(),
                max: MAX_CONFIG_BYTES,
            });
        }
        toml::from_str::<RawConfig>(text)
            .map_err(ConfigError::Parse)?
            .validate()
    }

    /// The language of the texts only the human reads.
    #[must_use]
    pub fn language(&self) -> &Language {
        &self.language
    }

    /// Every normal agent and worker, by name.
    #[must_use]
    pub fn agents(&self) -> &BTreeMap<AgentName, AgentConfig> {
        &self.agents
    }

    /// The agent named `name`, if there is one.
    #[must_use]
    pub fn agent(&self, name: &AgentName) -> Option<&AgentConfig> {
        self.agents.get(name)
    }

    /// The normal agents, in the order of the list.
    #[must_use]
    pub fn normal_agents(&self) -> &[AgentName] {
        &self.normal
    }

    /// The workers in the order of delegation: a request goes to the first free one.
    #[must_use]
    pub fn worker_agents(&self) -> &[AgentName] {
        &self.workers
    }

    /// The investigators, by name.
    #[must_use]
    pub fn investigators(&self) -> &BTreeMap<InvestigatorName, InvestigatorConfig> {
        &self.investigators
    }

    /// The investigator named `name`, if there is one.
    #[must_use]
    pub fn investigator(&self, name: &InvestigatorName) -> Option<&InvestigatorConfig> {
        self.investigators.get(name)
    }

    /// The programs and the agents' `PATH`.
    #[must_use]
    pub fn binaries(&self) -> &Binaries {
        &self.binaries
    }

    /// The hang watchdog.
    #[must_use]
    pub fn watchdog(&self) -> &Watchdog {
        &self.watchdog
    }

    /// The pauses of quotas and rate limits.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The review fix passes.
    #[must_use]
    pub fn review(&self) -> &Review {
        &self.review
    }

    /// The identity of the agents' commits.
    #[must_use]
    pub fn git(&self) -> &GitIdentity {
        &self.git
    }

    /// The trusted GitHub accounts.
    #[must_use]
    pub fn github(&self) -> &Github {
        &self.github
    }

    /// The notification command and directory.
    #[must_use]
    pub fn notifications(&self) -> &Notifications {
        &self.notifications
    }

    /// The dashboard's address and logins.
    #[must_use]
    pub fn web(&self) -> &Web {
        &self.web
    }

    /// The repository named `name`, if the configuration has it.
    #[must_use]
    pub fn repo(&self, name: &RepoName) -> Option<&RepoConfig> {
        self.repos.get(name)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A configuration that sets every setting to something other than its default.
    const FULL: &str = r#"
language = "Polish"

[agents.claude-1]
cli = "claude"
model = "sonnet"

[agents.codex-1]
cli = "codex"
args = ["-c", "model_reasoning_effort=\"medium\""]

[agents.glm]
cli = "opencode"
model = "deepinfra/zai-org/GLM-5.3-Flash"

[agents.qwen]
cli = "opencode"
model = "openrouter/@preset/qwen-worker"

[investigators.inv-opus]
cli = "claude"
model = "opus"

[investigators.inv-codex]
cli = "codex"
args = ["-c", "model_reasoning_effort=\"high\""]

[tiers]
normal = ["codex-1", "claude-1"]
workers = ["qwen", "glm"]

[binaries]
claude = "/opt/claude/bin/claude"
codex = "codex"
agy = "/opt/agy"
opencode = "/opt/opencode"
agentbus = "/usr/local/bin/agentbus"
serena = "serena"
agent_path = ["/opt/bin", "/usr/bin"]

[watchdog]
hang_minutes = 45
hang_strikes = 2

[limits]
quota_pause_minutes = 60
rate_limit_pause_minutes = 2
tail_lines = 50

[review]
fix_passes = 0

[git]
name = "{agent} żółć"
email = "{agent}@example.com"
signing_key = "/home/dev/.ssh/signing"

[github]
trusted_accounts = ["Octo-Cat", "ms-ai-agents"]

[notifications]
command = ["/usr/local/bin/notify", "--topic", "agents"]
directory = "/var/notify"

[web]
listen = "[::1]:9000"
allowed_logins = ["you@example.com", "Other@Example.com"]

[repos.team-agents]
base_branch = "main"
gate = ["make", "gate"]
gate_timeout_minutes = 45

[repos.other]
base_branch = "develop"
gate = ["/usr/bin/cargo", "test"]
"#;

    /// Two agents, one of each tier, and an investigator, for the cases that break one rule.
    const BASE: &str = r#"
[agents.claude-1]
cli = "claude"

[agents.qwen]
cli = "opencode"

[investigators.inv-opus]
cli = "claude"
"#;

    fn config(text: &str) -> Config {
        Config::from_toml_str(text).unwrap()
    }

    fn failure(text: &str) -> ConfigError {
        Config::from_toml_str(text).unwrap_err()
    }

    fn agent(name: &str) -> AgentName {
        name.parse().unwrap()
    }

    fn investigator(name: &str) -> InvestigatorName {
        name.parse().unwrap()
    }

    fn minutes(n: u64) -> Duration {
        Duration::from_secs(n * 60)
    }

    /// The line, counted from 1, of the byte `err` points at in `text`, if it is a parse error that points at one.
    fn error_line(text: &str, err: &ConfigError) -> Option<usize> {
        let ConfigError::Parse(source) = err else {
            panic!("not a parse error: {err:?}");
        };
        source
            .span()
            .map(|span| text.get(..span.start).unwrap().matches('\n').count() + 1)
    }

    /// The line, counted from 1, of the first line of `text` that is `marker`.
    fn line_of(text: &str, marker: &str) -> usize {
        text.lines().position(|line| line == marker).unwrap() + 1
    }

    #[test]
    fn an_empty_file_is_the_default_configuration() {
        for text in ["", "\n", "# only a comment\n"] {
            let config = config(text);
            assert!(config.language().is_english());
            assert_eq!(config.agents().len(), 0);
            assert_eq!(config.normal_agents(), Vec::<AgentName>::new());
            assert_eq!(config.worker_agents(), Vec::<AgentName>::new());
            assert_eq!(config.investigators().len(), 0);
            assert_eq!(config.binaries(), &Binaries::default());
            assert_eq!(config.watchdog().hang_minutes.as_duration(), minutes(30));
            assert_eq!(config.watchdog().hang_strikes.get(), 3);
            assert_eq!(config.limits().quota_pause_minutes.as_duration(), minutes(30));
            assert_eq!(config.limits().rate_limit_pause_minutes.as_duration(), minutes(5));
            assert_eq!(config.limits().tail_lines.get(), 30);
            assert_eq!(config.review().fix_passes, 5);
            assert_eq!(config.git(), &GitIdentity::default());
            assert_eq!(config.github().trusted_accounts, Vec::<GithubLogin>::new());
            assert_eq!(config.notifications(), &Notifications::default());
            assert_eq!(config.web().listen.to_string(), "127.0.0.1:8787");
            assert_eq!(config.web().allowed_logins, Vec::<TailscaleLogin>::new());
        }
    }

    #[test]
    fn the_defaults_are_what_the_documentation_says() {
        let binaries = Binaries::default();
        assert_eq!(binaries.claude.as_path().to_str(), Some("claude"));
        assert_eq!(binaries.agy.as_path().to_str(), Some("agy"));
        assert_eq!(binaries.serena, None);
        let path: Vec<_> = binaries
            .agent_path
            .iter()
            .map(|dir| dir.as_path().to_str().unwrap())
            .collect();
        assert_eq!(path, ["/usr/local/bin", "/usr/bin", "/bin"]);
        let git = GitIdentity::default();
        assert_eq!(git.name.render(&agent("claude-1")), "claude-1 (team-agents)");
        assert_eq!(git.email.render(&agent("claude-1")), "claude-1@team-agents.invalid");
        assert_eq!(git.signing_key, None);
    }

    #[test]
    fn a_full_file_is_read_into_its_parts() {
        let config = config(FULL);

        assert_eq!(config.language().as_str(), "Polish");
        assert_eq!(config.agents().len(), 4);
        let codex = config.agent(&agent("codex-1")).unwrap();
        assert_eq!(codex.cli, CliKind::Codex);
        assert_eq!(codex.model, None);
        assert_eq!(codex.args.as_slice(), ["-c", "model_reasoning_effort=\"medium\""]);
        let claude = config.agent(&agent("claude-1")).unwrap();
        assert_eq!(claude.model.as_ref().map(ModelName::as_str), Some("sonnet"));
        assert_eq!(claude.args, CliArgs::default());
        assert_eq!(config.agent(&agent("nobody")), None);

        let opus = config.investigator(&investigator("inv-opus")).unwrap();
        assert_eq!(
            (opus.cli, opus.model.as_ref().map(ModelName::as_str)),
            (CliKind::Claude, Some("opus"))
        );
        assert_eq!(config.investigator(&investigator("nobody")), None);

        assert_eq!(
            config.binaries().agentbus.as_path().to_str(),
            Some("/usr/local/bin/agentbus")
        );
        assert_eq!(
            config.binaries().serena.as_ref().map(|p| p.as_path().to_str()),
            Some(Some("serena"))
        );
        assert_eq!(config.binaries().agent_path.len(), 2);
        assert_eq!(config.watchdog().hang_minutes.as_duration(), minutes(45));
        assert_eq!(config.watchdog().hang_strikes.get(), 2);
        assert_eq!(config.limits().quota_pause_minutes.as_duration(), minutes(60));
        assert_eq!(config.limits().rate_limit_pause_minutes.as_duration(), minutes(2));
        assert_eq!(config.limits().tail_lines.get(), 50);
        assert_eq!(config.review().fix_passes, 0);
        assert_eq!(config.git().name.render(&agent("qwen")), "qwen żółć");
        assert_eq!(config.git().email.render(&agent("qwen")), "qwen@example.com");
        assert_eq!(
            config.git().signing_key.as_ref().unwrap().as_path().to_str(),
            Some("/home/dev/.ssh/signing")
        );
        let trusted: Vec<_> = config
            .github()
            .trusted_accounts
            .iter()
            .map(GithubLogin::as_str)
            .collect();
        assert_eq!(trusted, ["octo-cat", "ms-ai-agents"]);
        let notify = config.notifications().command.as_ref().unwrap();
        assert_eq!(notify.program().as_path().to_str(), Some("/usr/local/bin/notify"));
        assert_eq!(notify.args().as_slice(), ["--topic", "agents"]);
        assert_eq!(
            config.notifications().directory.as_ref().unwrap().as_path().to_str(),
            Some("/var/notify")
        );
        assert_eq!(config.web().listen.to_string(), "[::1]:9000");
        let logins: Vec<_> = config.web().allowed_logins.iter().map(TailscaleLogin::as_str).collect();
        assert_eq!(logins, ["you@example.com", "other@example.com"]);
    }

    #[test]
    fn the_order_of_the_lists_is_kept() {
        let config = config(FULL);
        assert_eq!(config.normal_agents(), [agent("codex-1"), agent("claude-1")]);
        // The order of delegation: the first free worker of this list gets the request.
        assert_eq!(config.worker_agents(), [agent("qwen"), agent("glm")]);
    }

    #[test]
    fn investigators_are_listed_by_name() {
        let names: Vec<_> = config(FULL)
            .investigators()
            .keys()
            .map(InvestigatorName::as_str)
            .collect::<Vec<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(names, ["inv-codex", "inv-opus"]);
    }

    #[test]
    fn each_cli_runs_its_own_program() {
        let config = config(FULL);
        let programs: Vec<_> = CliKind::ALL
            .iter()
            .map(|&kind| config.binaries().program(kind).as_path().to_str().unwrap())
            .collect();
        assert_eq!(
            programs,
            ["/opt/claude/bin/claude", "codex", "/opt/agy", "/opt/opencode"]
        );
    }

    #[test]
    fn a_repository_has_a_base_branch_a_gate_and_a_timeout() {
        let config = config(FULL);
        let repo = config.repo(&"team-agents".parse().unwrap()).unwrap();
        assert_eq!(repo.base_branch.as_str(), "main");
        assert_eq!(repo.gate.program().as_path().to_str(), Some("make"));
        assert_eq!(repo.gate.args().as_slice(), ["gate"]);
        assert_eq!(repo.gate_timeout_minutes.as_duration(), minutes(45));

        let other = config.repo(&"other".parse().unwrap()).unwrap();
        assert_eq!(other.base_branch.as_str(), "develop");
        assert_eq!(other.gate_timeout_minutes.as_duration(), minutes(30));

        assert_eq!(config.repo(&"unknown".parse().unwrap()), None);
    }

    #[test]
    fn a_list_naming_a_missing_agent_is_refused_with_the_list_and_the_name() {
        let cases = [
            (r#"normal = ["claude-1", "ghost"]"#, Tier::Normal, "ghost"),
            (
                r#"normal = ["claude-1"]
workers = ["ghost"]"#,
                Tier::Worker,
                "ghost",
            ),
            // An investigator is not an agent, so a list may not name one.
            (
                r#"normal = ["claude-1", "inv-opus"]
workers = ["qwen"]"#,
                Tier::Normal,
                "inv-opus",
            ),
        ];
        for (tiers, list, name) in cases {
            let text = format!("{BASE}\n[tiers]\n{tiers}\n");
            let err = failure(&text);
            assert!(
                matches!(&err, ConfigError::UnknownName { list: l, name: n } if *l == list && n.as_str() == name),
                "{tiers}: {err:?}"
            );
        }
    }

    #[test]
    fn a_list_naming_an_agent_twice_is_refused_with_the_list_and_the_name() {
        let cases = [
            (
                r#"normal = ["claude-1", "claude-1"]
workers = ["qwen"]"#,
                Tier::Normal,
                "claude-1",
            ),
            (
                r#"normal = ["claude-1"]
workers = ["qwen", "qwen"]"#,
                Tier::Worker,
                "qwen",
            ),
        ];
        for (tiers, list, name) in cases {
            let text = format!("{BASE}\n[tiers]\n{tiers}\n");
            let err = failure(&text);
            assert!(
                matches!(&err, ConfigError::DuplicateName { list: l, name: n } if *l == list && n.as_str() == name),
                "{tiers}: {err:?}"
            );
        }
    }

    #[test]
    fn a_name_in_both_lists_is_refused_with_that_name() {
        let text = format!("{BASE}\n[tiers]\nnormal = [\"claude-1\", \"qwen\"]\nworkers = [\"qwen\"]\n");
        let err = failure(&text);
        assert!(
            matches!(&err, ConfigError::NameInTwoLists { name, first: Tier::Normal, second: Tier::Worker } if name == "qwen"),
            "{err:?}"
        );
    }

    #[test]
    fn a_name_that_is_an_agent_and_an_investigator_is_refused_with_the_agents_list() {
        for (tiers, first) in [
            (
                "normal = [\"claude-1\", \"inv-opus\"]\nworkers = [\"qwen\"]",
                Tier::Normal,
            ),
            (
                "normal = [\"claude-1\"]\nworkers = [\"qwen\", \"inv-opus\"]",
                Tier::Worker,
            ),
        ] {
            let text = format!("{BASE}\n[agents.inv-opus]\ncli = \"claude\"\n\n[tiers]\n{tiers}\n");
            let err = failure(&text);
            assert!(
                matches!(&err, ConfigError::NameInTwoLists { name, first: f, second: Tier::Investigator } if name == "inv-opus" && *f == first),
                "{tiers}: {err:?}"
            );
        }
    }

    #[test]
    fn an_agent_in_no_list_is_refused_with_its_name() {
        let text = format!("{BASE}\n[tiers]\nnormal = [\"claude-1\"]\n");
        let err = failure(&text);
        assert!(
            matches!(&err, ConfigError::AgentInNoList { name } if name.as_str() == "qwen"),
            "{err:?}"
        );

        // No `[tiers]` at all: the agents are in no list either.
        let err = failure(BASE);
        assert!(matches!(&err, ConfigError::AgentInNoList { .. }), "{err:?}");
    }

    #[test]
    fn unknown_keys_are_refused_with_the_line_of_the_key() {
        let cases = [
            "lang = \"English\"",
            "[agents.a]\ncli = \"claude\"\nmodle = \"x\"",
            "[investigators.a]\ncli = \"claude\"\nmodle = \"x\"",
            "[tiers]\nnormal = []\nspare = []",
            "[binaries]\nclaud = \"claude\"",
            "[watchdog]\nhang_minute = 3",
            "[limits]\ntail = 3",
            "[review]\npasses = 3",
            "[git]\nemail_address = \"a@b\"",
            "[github]\ntrusted = []",
            "[notifications]\ncmd = [\"x\"]",
            "[web]\nlogins = []",
            "[repos.x]\nbase_branch = \"main\"\ngate = [\"make\"]\ntimeout = 3",
        ];
        for text in cases {
            let err = failure(text);
            let marker = text.lines().last().unwrap();
            assert_eq!(error_line(text, &err), Some(line_of(text, marker)), "{text:?}");
        }
    }

    #[test]
    fn an_unknown_section_is_refused_with_the_line_of_its_header() {
        let text = "language = \"English\"\n\n[unknown_section]\nx = 1\n";
        let err = failure(text);
        assert_eq!(error_line(text, &err), Some(3));
    }

    #[test]
    fn a_value_its_type_refuses_is_refused_with_the_line_of_the_value() {
        let cases = [
            "[agents.Bad_Name]\ncli = \"claude\"",
            "[agents.a]\ncli = \"gpt\"",
            "[agents.a]\ncli = \"claude\"\nmodel = \"two words\"",
            "[agents.a]\ncli = \"claude\"\nargs = \"-c x\"",
            "[agents.a]\ncli = \"claude\"\nargs = [1]",
            "[investigators.a]\ncli = \"\"",
            "[tiers]\nnormal = [\"Not Valid\"]",
            "[binaries]\nclaude = \"./claude\"",
            "[binaries]\nagent_path = [\"bin\"]",
            "[watchdog]\nhang_minutes = 0",
            "[watchdog]\nhang_strikes = 0",
            "[watchdog]\nhang_minutes = \"30\"",
            "[limits]\ntail_lines = -1",
            "[review]\nfix_passes = -1",
            "[review]\nfix_passes = \"5\"",
            "[git]\nname = \"{who}\"",
            "[git]\nsigning_key = \"id_ed25519\"",
            "[github]\ntrusted_accounts = [\"-x\"]",
            "[notifications]\ncommand = []",
            "[notifications]\ndirectory = \"notifications\"",
            "[web]\nlisten = \"0.0.0.0:8787\"",
            "[web]\nallowed_logins = [\"nobody\"]",
            "language = \"Brazilian Portuguese\"",
            "language = 5",
            "[repos.\"a/b\"]\nbase_branch = \"main\"\ngate = [\"make\"]",
            "[repos.x]\ngate = [\"make\"]\nbase_branch = \"bad branch\"",
            "[repos.x]\nbase_branch = \"main\"\ngate = [\"./make\"]",
            "[repos.x]\nbase_branch = \"main\"\ngate = [\"make\"]\ngate_timeout_minutes = 0",
        ];
        for text in cases {
            let err = failure(text);
            let marker = text.lines().last().unwrap();
            let line = if text.contains("[agents.Bad_Name]") || text.contains("[repos.\"a/b\"]") {
                1
            } else {
                line_of(text, marker)
            };
            assert_eq!(error_line(text, &err), Some(line), "{text:?}");
        }
    }

    #[test]
    fn a_repository_without_its_gate_or_base_branch_is_refused() {
        for text in [
            "[repos.x]\nbase_branch = \"main\"",
            "[repos.x]\ngate = [\"make\"]",
            "[agents.a]\ncliii = \"claude\"",
            "[agents.a]\nmodel = \"x\"",
        ] {
            let err = failure(text);
            assert!(matches!(err, ConfigError::Parse(_)), "{text:?}");
        }
    }

    #[test]
    fn text_that_is_not_toml_is_a_parse_error_with_its_line() {
        for (text, line) in [
            ("language = \n", 1),
            ("language = \"English\"\n[agents\n", 2),
            ("language = \"English\"\nlanguage = \"Polish\"\n", 2),
            ("[agents.a]\ncli = \"claude\"\n[agents.a]\ncli = \"claude\"\n", 3),
            ("\u{0}", 1),
        ] {
            let err = failure(text);
            assert_eq!(error_line(text, &err), Some(line), "{text:?}");
        }
    }

    #[test]
    fn a_text_over_the_limit_is_refused_and_one_at_the_limit_is_read() {
        let at_the_limit = "#".repeat(MAX_CONFIG_BYTES);
        assert_eq!(Config::from_toml_str(&at_the_limit).unwrap().agents().len(), 0);

        let over = format!("{at_the_limit}#");
        let err = failure(&over);
        assert!(
            matches!(err, ConfigError::TooLarge { len, max } if len == MAX_CONFIG_BYTES + 1 && max == MAX_CONFIG_BYTES),
            "{err:?}"
        );
    }

    #[test]
    fn non_ascii_text_in_values_and_comments_is_read() {
        let config = config("# zażółć gęślą jaźń\n[git]\nname = \"{agent} ✓\"\n");
        assert_eq!(config.git().name.render(&agent("a")), "a ✓");
    }
}
