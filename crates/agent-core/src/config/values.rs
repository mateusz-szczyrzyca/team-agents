//! The typed values of `agents.toml` and `secrets.toml`: each is a newtype that can hold only a valid value (TYP-1,
//! TYP-2). A value is made by parsing — [`FromStr`], [`TryFrom`] and serde all run the same check — and a refused one
//! is an [`InvalidValue`] that names what was parsed, the text and the [`ValueRule`] it breaks.
//!
//! The checks look at the text only. Whether a program exists or a path is readable is for the runtime to find out.

// `core::net`, not `std::net`: these types parse and compare addresses and open no socket, and `agent-core` has no
// I/O (docs/architecture.md).
use core::net::SocketAddr;
use std::fmt;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;

use crate::ids::AgentName;

/// Longest language name, in bytes: it names a directory of prompt overrides.
const MAX_LANGUAGE_LEN: usize = 32;

/// Longest model name, in bytes.
const MAX_MODEL_LEN: usize = 200;

/// Longest repository name, in bytes: GitHub's own limit.
const MAX_REPO_LEN: usize = 100;

/// Longest environment variable name, in bytes.
const MAX_ENV_NAME_LEN: usize = 128;

/// Longest GitHub login, in bytes: GitHub's own limit.
const MAX_GITHUB_LOGIN_LEN: usize = 39;

/// Longest Tailscale login, in bytes: the limit of an e-mail address.
const MAX_TAILSCALE_LOGIN_LEN: usize = 254;

/// Longest name template, in bytes.
const MAX_TEMPLATE_LEN: usize = 200;

/// Longest path or program, in bytes: Linux's `PATH_MAX`.
const MAX_PATH_LEN: usize = 4096;

/// Most arguments a command line may have.
const MAX_ARGS: usize = 64;

/// Longest single argument, in bytes.
const MAX_ARG_LEN: usize = 4096;

/// The placeholder of a [`NameTemplate`] that stands for the agent's name.
const AGENT_PLACEHOLDER: &str = "{agent}";

/// Seconds in a minute, for [`Minutes::as_duration`].
const SECS_PER_MINUTE: u64 = 60;

/// Why a text of a configuration file is not a valid value: what it was parsed as, the refused text and the rule it
/// breaks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what} {value:?}: {rule}")]
pub struct InvalidValue {
    /// What the text was parsed as, for the message: `language`, `GitHub login`, ...
    pub what: &'static str,
    /// The refused text; for a list of arguments, the arguments joined by spaces.
    pub value: String,
    /// The rule the text breaks.
    pub rule: ValueRule,
}

/// The rule of a configuration value that a text breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ValueRule {
    /// The text is empty.
    #[error("it is empty")]
    Empty,
    /// The text is longer than `max` bytes.
    #[error("it is longer than {max} bytes")]
    TooLong {
        /// The longest allowed text, in bytes.
        max: usize,
    },
    /// The list has more than `max` elements.
    #[error("it has more than {max} elements")]
    TooMany {
        /// The most elements allowed.
        max: usize,
    },
    /// The text contains this character, which the value may not.
    #[error("{0:?} is not allowed")]
    ForbiddenChar(char),
    /// The text uses braces other than the placeholder `{agent}`.
    #[error("only the placeholder {{agent}} may use braces")]
    StrayBrace,
    /// The name starts with a digit.
    #[error("it must not start with a digit")]
    StartsWithDigit,
    /// The login starts or ends with `-`.
    #[error("it must not start or end with '-'")]
    HyphenAtEdge,
    /// The login has two `-` in a row.
    #[error("it must not have two '-' in a row")]
    DoubleHyphen,
    /// The name is `.` or `..`.
    #[error("it must not be \".\" or \"..\"")]
    DotName,
    /// The text is not of the form `user@example.com`.
    #[error("it must look like user@example.com")]
    NotLogin,
    /// The path is not absolute.
    #[error("it must be an absolute path")]
    NotAbsolute,
    /// The program is neither a bare command name nor an absolute path.
    #[error("it must be a bare command name or an absolute path")]
    NotBareOrAbsolute,
    /// The text is not an `address:port` pair of IP address and port.
    #[error("it must be an IP address and a port, like 127.0.0.1:8787")]
    NotSocketAddr,
    /// The address is not a loopback address.
    #[error("it must be a loopback address")]
    NotLoopback,
    /// The port is 0.
    #[error("the port must not be 0")]
    ZeroPort,
}

/// An [`InvalidValue`] for `text`.
fn refuse(what: &'static str, text: &str, rule: ValueRule) -> InvalidValue {
    InvalidValue {
        what,
        value: text.to_owned(),
        rule,
    }
}

/// Checks that `text` is not empty, has at most `max` bytes and only characters `allowed` accepts.
fn check_text(what: &'static str, text: &str, max: usize, allowed: impl Fn(char) -> bool) -> Result<(), InvalidValue> {
    if text.is_empty() {
        return Err(refuse(what, text, ValueRule::Empty));
    }
    if text.len() > max {
        return Err(refuse(what, text, ValueRule::TooLong { max }));
    }
    match text.chars().find(|&c| !allowed(c)) {
        Some(bad) => Err(refuse(what, text, ValueRule::ForbiddenChar(bad))),
        None => Ok(()),
    }
}

/// Implements `as_str`, `Display`, `FromStr` and `TryFrom<String>` (which serde deserializes through) for a text
/// newtype `$ty(String)`: both conversions run `$check`, then `$normalize` makes the stored text.
macro_rules! text_value {
    ($ty:ident, $check:path, $normalize:path) => {
        impl $ty {
            /// The value's text, as it is stored.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $ty {
            type Err = InvalidValue;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                $check(text)?;
                Ok(Self($normalize(text.to_owned())))
            }
        }

        impl TryFrom<String> for $ty {
            type Error = InvalidValue;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                $check(&text)?;
                Ok(Self($normalize(text)))
            }
        }
    };
}

/// Keeps a text as it is.
fn keep(text: String) -> String {
    text
}

/// Makes a text lowercase: GitHub and Tailscale logins do not distinguish case.
fn lowercase(mut text: String) -> String {
    text.make_ascii_lowercase();
    text
}

/// The language of the texts only the human reads — questions, notifications —, such as `English` or `Polish`.
///
/// 1 to 32 bytes of ASCII letters, digits and `-`, so that it can name a directory of prompt overrides. Refused,
/// with the [`ValueRule`] it breaks: the empty text, a longer one and any other character (spaces, `.`, `/`,
/// non-ASCII letters). The default is `English`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct Language(String);

fn check_language(text: &str) -> Result<(), InvalidValue> {
    check_text("language", text, MAX_LANGUAGE_LEN, |c| {
        c.is_ascii_alphanumeric() || c == '-'
    })
}

text_value!(Language, check_language, keep);

impl Language {
    /// Whether this is English, the language every text exists in; any case.
    #[must_use]
    pub fn is_english(&self) -> bool {
        self.0.eq_ignore_ascii_case("english")
    }

    /// A language written as a literal in this crate; the test `defaults_pass_their_own_checks` keeps it valid.
    pub(crate) fn literal(text: &'static str) -> Self {
        Self(text.to_owned())
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::literal("English")
    }
}

/// The model an agent runs, as its CLI takes it: `sonnet`, `openrouter/@preset/qwen-worker`.
///
/// 1 to 200 bytes without whitespace or control characters. Refused, with the [`ValueRule`] it breaks: the empty
/// text, a longer one, whitespace and control characters. An agent without a model uses its CLI's default.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct ModelName(String);

fn check_model(text: &str) -> Result<(), InvalidValue> {
    check_text("model name", text, MAX_MODEL_LEN, |c| {
        !c.is_control() && !c.is_whitespace()
    })
}

text_value!(ModelName, check_model, keep);

/// The name of a repository in `[repos.NAME]`, as GitHub names it: `team-agents`.
///
/// 1 to 100 bytes of ASCII letters, digits, `-`, `_` and `.`, other than `.` and `..`. Refused, with the
/// [`ValueRule`] it breaks: the empty text, a longer one, any other character and the two dot names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct RepoName(String);

fn check_repo(text: &str) -> Result<(), InvalidValue> {
    check_text("repository name", text, MAX_REPO_LEN, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
    })?;
    if matches!(text, "." | "..") {
        return Err(refuse("repository name", text, ValueRule::DotName));
    }
    Ok(())
}

text_value!(RepoName, check_repo, keep);

/// The name of an environment variable in `secrets.toml`: `ANTHROPIC_API_KEY`.
///
/// 1 to 128 bytes of ASCII letters, digits and `_`, not starting with a digit. Refused, with the [`ValueRule`] it
/// breaks: the empty text, a longer one, a first digit and any other character.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct EnvName(String);

fn check_env_name(text: &str) -> Result<(), InvalidValue> {
    check_text("environment variable name", text, MAX_ENV_NAME_LEN, |c| {
        c.is_ascii_alphanumeric() || c == '_'
    })?;
    if text.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(refuse("environment variable name", text, ValueRule::StartsWithDigit));
    }
    Ok(())
}

text_value!(EnvName, check_env_name, keep);

/// A GitHub login, such as `ms-ai-agents`, kept in lowercase because GitHub does not distinguish case.
///
/// 1 to 39 bytes of ASCII letters, digits and `-`, not starting or ending with `-` and without `--`. Refused, with
/// the [`ValueRule`] it breaks: the empty text, a longer one, any other character, a `-` at an edge and two in a row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct GithubLogin(String);

fn check_github_login(text: &str) -> Result<(), InvalidValue> {
    check_text("GitHub login", text, MAX_GITHUB_LOGIN_LEN, |c| {
        c.is_ascii_alphanumeric() || c == '-'
    })?;
    if text.starts_with('-') || text.ends_with('-') {
        return Err(refuse("GitHub login", text, ValueRule::HyphenAtEdge));
    }
    if text.contains("--") {
        return Err(refuse("GitHub login", text, ValueRule::DoubleHyphen));
    }
    Ok(())
}

text_value!(GithubLogin, check_github_login, lowercase);

/// A Tailscale login, such as `you@example.com`, kept in lowercase.
///
/// Up to 254 bytes of the form `user@domain`, without whitespace, control characters, `<` or `>`. Refused, with the
/// [`ValueRule`] it breaks: the empty text, a longer one, those characters and anything that is not one `@` between
/// two non-empty parts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct TailscaleLogin(String);

fn check_tailscale_login(text: &str) -> Result<(), InvalidValue> {
    check_text("Tailscale login", text, MAX_TAILSCALE_LOGIN_LEN, |c| {
        !c.is_control() && !c.is_whitespace() && c != '<' && c != '>'
    })?;
    match text.split_once('@') {
        Some((user, domain)) if !user.is_empty() && !domain.is_empty() && !domain.contains('@') => Ok(()),
        _ => Err(refuse("Tailscale login", text, ValueRule::NotLogin)),
    }
}

text_value!(TailscaleLogin, check_tailscale_login, lowercase);

/// A text with the placeholder `{agent}` for an agent's name, such as `{agent} (team-agents)`: the name and the
/// e-mail of the commits an agent makes.
///
/// 1 to 200 bytes without control characters, `<` or `>`; the only braces allowed are those of `{agent}`. Refused,
/// with the [`ValueRule`] it breaks: the empty text, a longer one, those characters and any other brace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct NameTemplate(String);

fn check_template(text: &str) -> Result<(), InvalidValue> {
    check_text("name template", text, MAX_TEMPLATE_LEN, |c| {
        !c.is_control() && c != '<' && c != '>'
    })?;
    if text.replace(AGENT_PLACEHOLDER, "").contains(['{', '}']) {
        return Err(refuse("name template", text, ValueRule::StrayBrace));
    }
    Ok(())
}

text_value!(NameTemplate, check_template, keep);

impl NameTemplate {
    /// The text with every `{agent}` replaced by `agent`'s name.
    #[must_use]
    pub fn render(&self, agent: &AgentName) -> String {
        self.0.replace(AGENT_PLACEHOLDER, agent.as_str())
    }

    /// A template written as a literal in this crate; the test `defaults_pass_their_own_checks` keeps it valid.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the defaults of the configuration sections")
    )]
    pub(crate) fn literal(text: &'static str) -> Self {
        Self(text.to_owned())
    }
}

/// A path that starts at the root, such as `/home/you/.local/bin`: the configuration never means a path relative to
/// whatever directory the daemon runs in.
///
/// 1 to 4096 bytes without NUL. Refused, with the [`ValueRule`] it breaks: the empty text, a longer one, a NUL and a
/// relative path (`bin/x`, `./x`, `~/x`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct AbsolutePath(PathBuf);

fn check_absolute(text: &str) -> Result<(), InvalidValue> {
    check_text("path", text, MAX_PATH_LEN, |c| c != '\0')?;
    if !Path::new(text).is_absolute() {
        return Err(refuse("path", text, ValueRule::NotAbsolute));
    }
    Ok(())
}

impl AbsolutePath {
    /// The path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// A path written as a literal in this crate; the test `defaults_pass_their_own_checks` keeps it valid.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the defaults of the configuration sections")
    )]
    pub(crate) fn literal(text: &'static str) -> Self {
        Self(PathBuf::from(text))
    }
}

impl FromStr for AbsolutePath {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        check_absolute(text)?;
        Ok(Self(PathBuf::from(text)))
    }
}

impl TryFrom<String> for AbsolutePath {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        check_absolute(&text)?;
        Ok(Self(PathBuf::from(text)))
    }
}

/// A program to run: a bare command name such as `git`, which is looked up on the agents' `PATH`, or an absolute path.
///
/// 1 to 4096 bytes without NUL. Refused, with the [`ValueRule`] it breaks: the empty text, a longer one, a NUL, and a
/// path that is not absolute but has a `/` (`./x`, `bin/x`), and `.` and `..`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct Program(PathBuf);

fn check_program(text: &str) -> Result<(), InvalidValue> {
    check_text("program", text, MAX_PATH_LEN, |c| c != '\0')?;
    let has_separator = text.contains('/');
    let usable = if has_separator {
        Path::new(text).is_absolute()
    } else {
        !matches!(text, "." | "..")
    };
    if !usable {
        return Err(refuse("program", text, ValueRule::NotBareOrAbsolute));
    }
    Ok(())
}

impl Program {
    /// The program, as a command name or an absolute path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Whether this is a bare command name, to be looked up on `PATH`, rather than an absolute path.
    #[must_use]
    pub fn is_bare_name(&self) -> bool {
        !self.0.is_absolute()
    }

    /// A program written as a literal in this crate; the test `defaults_pass_their_own_checks` keeps it valid.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the defaults of the configuration sections")
    )]
    pub(crate) fn literal(text: &'static str) -> Self {
        Self(PathBuf::from(text))
    }
}

impl FromStr for Program {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        check_program(text)?;
        Ok(Self(PathBuf::from(text)))
    }
}

impl TryFrom<String> for Program {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        check_program(&text)?;
        Ok(Self(PathBuf::from(text)))
    }
}

/// Extra arguments for a program: the `args` of an agent in the configuration.
///
/// At most 64 arguments of at most 4096 bytes each, without NUL; an argument may be empty. Refused, with the
/// [`ValueRule`] it breaks: more arguments, a longer argument and a NUL. They reach the program as they are, never
/// through a shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct CliArgs(Vec<String>);

/// Checks the arguments `args` of a command line.
fn check_args(args: &[String]) -> Result<(), InvalidValue> {
    let refuse_args = |rule| Err(refuse("argument", &args.join(" "), rule));
    if args.len() > MAX_ARGS {
        return refuse_args(ValueRule::TooMany { max: MAX_ARGS });
    }
    if args.iter().any(|arg| arg.len() > MAX_ARG_LEN) {
        return refuse_args(ValueRule::TooLong { max: MAX_ARG_LEN });
    }
    if args.iter().any(|arg| arg.contains('\0')) {
        return refuse_args(ValueRule::ForbiddenChar('\0'));
    }
    Ok(())
}

impl CliArgs {
    /// The arguments, in order.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl TryFrom<Vec<String>> for CliArgs {
    type Error = InvalidValue;

    fn try_from(args: Vec<String>) -> Result<Self, Self::Error> {
        check_args(&args)?;
        Ok(Self(args))
    }
}

/// A program and its arguments, written in the configuration as one array: `["make", "gate"]`.
///
/// The first element is the [`Program`], the rest are [`CliArgs`]; refused with the rules of those, and with
/// [`ValueRule::Empty`] for the empty array.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct CommandLine {
    /// What to run.
    program: Program,
    /// What to pass to it.
    args: CliArgs,
}

impl CommandLine {
    /// What to run.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// The arguments after the program.
    #[must_use]
    pub fn args(&self) -> &CliArgs {
        &self.args
    }
}

impl TryFrom<Vec<String>> for CommandLine {
    type Error = InvalidValue;

    fn try_from(words: Vec<String>) -> Result<Self, Self::Error> {
        let mut words = words.into_iter();
        let Some(program) = words.next() else {
            return Err(refuse("command", "", ValueRule::Empty));
        };
        Ok(Self {
            program: Program::try_from(program)?,
            args: CliArgs::try_from(words.collect::<Vec<_>>())?,
        })
    }
}

/// A number of minutes, never 0: a watchdog limit, a pause, a timeout. A zero or a negative number is refused when
/// it is deserialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(transparent)]
pub struct Minutes(NonZeroU32);

impl Minutes {
    /// A number of minutes written as a literal in this crate.
    #[expect(dead_code, reason = "used by the defaults of the configuration sections")]
    pub(crate) const fn literal(minutes: NonZeroU32) -> Self {
        Self(minutes)
    }

    /// The span of time, in whole seconds.
    #[must_use]
    pub fn as_duration(self) -> Duration {
        // A u32 count of minutes in seconds is below 2^38: it fits a u64.
        Duration::from_secs(u64::from(self.0.get()) * SECS_PER_MINUTE)
    }
}

/// An address the dashboard listens on: an IP address and a port, the address a loopback one and the port not 0.
/// Remote access goes through `tailscale serve`, never through a listening socket on the network.
///
/// Refused, with the [`ValueRule`] it breaks: a text that is not `address:port` (a host name, a missing port), an
/// address that is not loopback (`0.0.0.0`, `::`, a LAN or Tailscale address) and port 0. The default is
/// `127.0.0.1:8787`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct LoopbackAddr(SocketAddr);

/// Checks that `text` is a loopback `address:port` and returns it.
fn parse_loopback(text: &str) -> Result<SocketAddr, InvalidValue> {
    // The parse error says no more than `NotSocketAddr` and the refused text do.
    let Ok(addr) = text.parse::<SocketAddr>() else {
        return Err(refuse("listen address", text, ValueRule::NotSocketAddr));
    };
    if !addr.ip().is_loopback() {
        return Err(refuse("listen address", text, ValueRule::NotLoopback));
    }
    if addr.port() == 0 {
        return Err(refuse("listen address", text, ValueRule::ZeroPort));
    }
    Ok(addr)
}

impl LoopbackAddr {
    /// The address and port.
    #[must_use]
    pub fn as_socket_addr(self) -> SocketAddr {
        self.0
    }
}

impl Default for LoopbackAddr {
    fn default() -> Self {
        Self(SocketAddr::from(([127, 0, 0, 1], 8787)))
    }
}

impl fmt::Display for LoopbackAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for LoopbackAddr {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_loopback(text).map(Self)
    }
}

impl TryFrom<String> for LoopbackAddr {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        parse_loopback(&text).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use serde::de::DeserializeOwned;
    use serde_json::error::Category;

    use super::*;

    /// Parses `text` as a `T` through `FromStr`, `TryFrom<String>` and serde, asserts that all agree and returns it.
    fn accepts<T>(text: &str) -> T
    where
        T: FromStr<Err = InvalidValue> + TryFrom<String, Error = InvalidValue> + DeserializeOwned + PartialEq + Debug,
    {
        let parsed: T = text.parse().unwrap();
        assert_eq!(T::try_from(text.to_owned()).unwrap(), parsed, "{text:?}");
        let json = serde_json::to_string(text).unwrap();
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), parsed, "{text:?}");
        parsed
    }

    /// Asserts that `T` refuses `text` as `what`, breaking `rule`, through `FromStr`, `TryFrom<String>` and serde.
    fn refuses<T>(what: &'static str, text: &str, rule: ValueRule)
    where
        T: FromStr<Err = InvalidValue> + TryFrom<String, Error = InvalidValue> + DeserializeOwned + Debug,
    {
        let expected = InvalidValue {
            what,
            value: text.to_owned(),
            rule,
        };
        assert_eq!(text.parse::<T>().unwrap_err(), expected, "{text:?}");
        assert_eq!(T::try_from(text.to_owned()).unwrap_err(), expected, "{text:?}");
        let json = serde_json::to_string(text).unwrap();
        let err = serde_json::from_str::<T>(&json).unwrap_err();
        assert_eq!(err.classify(), Category::Data, "{text:?}");
    }

    /// `n` copies of `c` as a text.
    fn repeated(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    #[test]
    fn language_accepts_names_and_codes() {
        for text in [
            "English",
            "Polish",
            "pt-BR",
            "de",
            "Brazilian-Portuguese",
            &repeated('a', 32),
        ] {
            assert_eq!(accepts::<Language>(text).as_str(), text);
        }
    }

    #[test]
    fn language_refuses_what_cannot_name_a_directory() {
        let long = repeated('a', 33);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 32 }),
            ("Brazilian Portuguese", ValueRule::ForbiddenChar(' ')),
            ("../x", ValueRule::ForbiddenChar('.')),
            ("a/b", ValueRule::ForbiddenChar('/')),
            ("żółw", ValueRule::ForbiddenChar('ż')),
            ("日本語", ValueRule::ForbiddenChar('日')),
            ("a\nb", ValueRule::ForbiddenChar('\n')),
        ];
        for (text, rule) in cases {
            refuses::<Language>("language", text, rule);
        }
    }

    #[test]
    fn language_defaults_to_english_and_knows_english_in_any_case() {
        assert_eq!(Language::default(), accepts::<Language>("English"));
        for text in ["English", "english", "ENGLISH"] {
            assert!(accepts::<Language>(text).is_english(), "{text:?}");
        }
        for text in ["Polish", "en", "Englishx"] {
            assert!(!accepts::<Language>(text).is_english(), "{text:?}");
        }
    }

    #[test]
    fn model_name_accepts_the_names_clis_take() {
        let long = repeated('m', 200);
        for text in [
            "sonnet",
            "gpt-5.5",
            "openrouter/@preset/qwen-worker",
            "deepinfra/zai-org/GLM-5.3-Flash",
            "модель",
            long.as_str(),
        ] {
            assert_eq!(accepts::<ModelName>(text).as_str(), text);
        }
    }

    #[test]
    fn model_name_refuses_whitespace_and_control_characters() {
        let long = repeated('m', 201);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 200 }),
            ("a b", ValueRule::ForbiddenChar(' ')),
            ("a\nb", ValueRule::ForbiddenChar('\n')),
            ("a\tb", ValueRule::ForbiddenChar('\t')),
            ("a\u{a0}b", ValueRule::ForbiddenChar('\u{a0}')),
            ("a\0b", ValueRule::ForbiddenChar('\0')),
        ];
        for (text, rule) in cases {
            refuses::<ModelName>("model name", text, rule);
        }
    }

    #[test]
    fn repo_name_accepts_what_github_names_a_repository() {
        for text in ["team-agents", "my.repo_2", "a", ".github", "...", &repeated('r', 100)] {
            assert_eq!(accepts::<RepoName>(text).as_str(), text);
        }
    }

    #[test]
    fn repo_name_refuses_paths_and_dot_names() {
        let long = repeated('r', 101);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 100 }),
            (".", ValueRule::DotName),
            ("..", ValueRule::DotName),
            ("a/b", ValueRule::ForbiddenChar('/')),
            ("a b", ValueRule::ForbiddenChar(' ')),
            ("żółw", ValueRule::ForbiddenChar('ż')),
        ];
        for (text, rule) in cases {
            refuses::<RepoName>("repository name", text, rule);
        }
    }

    #[test]
    fn env_name_accepts_shell_variable_names() {
        for text in ["ANTHROPIC_API_KEY", "_x", "a1", "X", &repeated('A', 128)] {
            assert_eq!(accepts::<EnvName>(text).as_str(), text);
        }
    }

    #[test]
    fn env_name_refuses_what_a_process_environment_cannot_name() {
        let long = repeated('A', 129);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 128 }),
            ("1A", ValueRule::StartsWithDigit),
            ("A-B", ValueRule::ForbiddenChar('-')),
            ("A B", ValueRule::ForbiddenChar(' ')),
            ("A=B", ValueRule::ForbiddenChar('=')),
            ("ŻÓŁW", ValueRule::ForbiddenChar('Ż')),
        ];
        for (text, rule) in cases {
            refuses::<EnvName>("environment variable name", text, rule);
        }
    }

    #[test]
    fn github_login_is_kept_in_lowercase() {
        for (text, stored) in [
            ("ms-ai-agents", "ms-ai-agents"),
            ("Octo-Cat", "octo-cat"),
            ("A", "a"),
            ("a1-b2", "a1-b2"),
        ] {
            assert_eq!(accepts::<GithubLogin>(text).as_str(), stored);
        }
    }

    #[test]
    fn github_login_refuses_what_github_does() {
        let long = repeated('a', 40);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 39 }),
            ("-a", ValueRule::HyphenAtEdge),
            ("a-", ValueRule::HyphenAtEdge),
            ("-", ValueRule::HyphenAtEdge),
            ("a--b", ValueRule::DoubleHyphen),
            ("a_b", ValueRule::ForbiddenChar('_')),
            ("a b", ValueRule::ForbiddenChar(' ')),
            ("a@b", ValueRule::ForbiddenChar('@')),
            ("żółw", ValueRule::ForbiddenChar('ż')),
        ];
        for (text, rule) in cases {
            refuses::<GithubLogin>("GitHub login", text, rule);
        }
    }

    #[test]
    fn github_login_accepts_the_longest_login() {
        let longest = repeated('a', 39);
        assert_eq!(accepts::<GithubLogin>(&longest).as_str(), longest);
    }

    #[test]
    fn tailscale_login_is_kept_in_lowercase() {
        for (text, stored) in [
            ("you@example.com", "you@example.com"),
            ("User@Example.COM", "user@example.com"),
            ("alice@github", "alice@github"),
        ] {
            assert_eq!(accepts::<TailscaleLogin>(text).as_str(), stored);
        }
    }

    #[test]
    fn tailscale_login_refuses_what_is_not_user_at_domain() {
        let long = repeated('a', 255);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 254 }),
            ("you", ValueRule::NotLogin),
            ("@example.com", ValueRule::NotLogin),
            ("you@", ValueRule::NotLogin),
            ("a@b@c", ValueRule::NotLogin),
            ("a b@c", ValueRule::ForbiddenChar(' ')),
            ("<a@b>", ValueRule::ForbiddenChar('<')),
            ("a@b\n", ValueRule::ForbiddenChar('\n')),
        ];
        for (text, rule) in cases {
            refuses::<TailscaleLogin>("Tailscale login", text, rule);
        }
    }

    #[test]
    fn name_template_accepts_text_with_and_without_the_placeholder() {
        for text in [
            "{agent} (team-agents)",
            "{agent}@team-agents.invalid",
            "{agent}-{agent}",
            "team-agents",
            "Zażółć {agent}",
        ] {
            assert_eq!(accepts::<NameTemplate>(text).as_str(), text);
        }
    }

    #[test]
    fn name_template_refuses_other_braces_and_what_breaks_a_git_identity() {
        let long = repeated('t', 201);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 200 }),
            ("{who}", ValueRule::StrayBrace),
            ("{agent", ValueRule::StrayBrace),
            ("agent}", ValueRule::StrayBrace),
            ("{{agent}}", ValueRule::StrayBrace),
            ("{}", ValueRule::StrayBrace),
            ("a<b", ValueRule::ForbiddenChar('<')),
            ("a>b", ValueRule::ForbiddenChar('>')),
            ("a\nb", ValueRule::ForbiddenChar('\n')),
        ];
        for (text, rule) in cases {
            refuses::<NameTemplate>("name template", text, rule);
        }
    }

    #[test]
    fn name_template_renders_the_agent_name_everywhere_the_placeholder_is() {
        let agent: AgentName = "claude-1".parse().unwrap();
        for (template, rendered) in [
            ("{agent} (team-agents)", "claude-1 (team-agents)"),
            ("{agent}-{agent}", "claude-1-claude-1"),
            ("team-agents", "team-agents"),
            ("{agent}", "claude-1"),
        ] {
            assert_eq!(accepts::<NameTemplate>(template).render(&agent), rendered);
        }
    }

    #[test]
    fn absolute_path_accepts_paths_from_the_root() {
        let long = format!("/{}", repeated('a', 4095));
        for text in ["/", "/home/you/.local/bin", "/a/../b", "/with space/x", long.as_str()] {
            assert_eq!(accepts::<AbsolutePath>(text).as_path(), Path::new(text));
        }
    }

    #[test]
    fn absolute_path_refuses_relative_paths_and_nul() {
        let long = format!("/{}", repeated('a', 4096));
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 4096 }),
            ("bin/x", ValueRule::NotAbsolute),
            ("./x", ValueRule::NotAbsolute),
            ("../x", ValueRule::NotAbsolute),
            ("~/x", ValueRule::NotAbsolute),
            ("x", ValueRule::NotAbsolute),
            ("/a\0b", ValueRule::ForbiddenChar('\0')),
        ];
        for (text, rule) in cases {
            refuses::<AbsolutePath>("path", text, rule);
        }
    }

    #[test]
    fn program_is_a_bare_name_or_an_absolute_path() {
        for (text, bare) in [
            ("claude", true),
            ("git", true),
            ("my-tool.sh", true),
            ("/usr/bin/claude", false),
            ("/home/you/.local/bin/agy", false),
        ] {
            let program = accepts::<Program>(text);
            assert_eq!(program.as_path(), Path::new(text));
            assert_eq!(program.is_bare_name(), bare, "{text:?}");
        }
    }

    #[test]
    fn program_refuses_relative_paths_dots_and_nul() {
        let long = repeated('p', 4097);
        let cases = [
            ("", ValueRule::Empty),
            (long.as_str(), ValueRule::TooLong { max: 4096 }),
            ("./claude", ValueRule::NotBareOrAbsolute),
            ("bin/claude", ValueRule::NotBareOrAbsolute),
            ("../claude", ValueRule::NotBareOrAbsolute),
            ("~/claude", ValueRule::NotBareOrAbsolute),
            (".", ValueRule::NotBareOrAbsolute),
            ("..", ValueRule::NotBareOrAbsolute),
            ("a\0b", ValueRule::ForbiddenChar('\0')),
        ];
        for (text, rule) in cases {
            refuses::<Program>("program", text, rule);
        }
    }

    /// A list of `n` arguments `a`.
    fn args(n: usize) -> Vec<String> {
        vec!["a".to_owned(); n]
    }

    #[test]
    fn cli_args_accept_any_arguments_within_the_bounds() {
        let longest = vec![repeated('x', 4096)];
        for words in [
            vec![],
            vec![String::new()],
            vec!["-c".to_owned(), "model_reasoning_effort=\"high\"".to_owned()],
            vec!["--flag with spaces".to_owned(), "żółw".to_owned()],
            args(64),
            longest,
        ] {
            let parsed = CliArgs::try_from(words.clone()).unwrap();
            assert_eq!(parsed.as_slice(), words.as_slice());
            assert_eq!(
                serde_json::from_str::<CliArgs>(&serde_json::to_string(&words).unwrap()).unwrap(),
                parsed
            );
        }
    }

    #[test]
    fn cli_args_refuse_too_many_too_long_and_nul() {
        let cases = [
            (args(65), ValueRule::TooMany { max: 64 }),
            (vec![repeated('x', 4097)], ValueRule::TooLong { max: 4096 }),
            (vec!["a".to_owned(), "b\0c".to_owned()], ValueRule::ForbiddenChar('\0')),
        ];
        for (words, rule) in cases {
            let expected = InvalidValue {
                what: "argument",
                value: words.join(" "),
                rule,
            };
            assert_eq!(CliArgs::try_from(words.clone()).unwrap_err(), expected);
            let err = serde_json::from_str::<CliArgs>(&serde_json::to_string(&words).unwrap()).unwrap_err();
            assert_eq!(err.classify(), Category::Data);
        }
    }

    #[test]
    fn command_line_is_a_program_and_its_arguments() {
        let words: Vec<String> = vec!["make".into(), "gate".into(), "-j4".into()];
        let line = CommandLine::try_from(words.clone()).unwrap();
        assert_eq!(line.program(), &accepts::<Program>("make"));
        assert_eq!(line.args().as_slice(), &words[1..]);

        let alone = CommandLine::try_from(vec!["/usr/local/bin/notify".to_owned()]).unwrap();
        assert_eq!(alone.program(), &accepts::<Program>("/usr/local/bin/notify"));
        assert_eq!(alone.args(), &CliArgs::default());

        let from_json: CommandLine = serde_json::from_str(r#"["make","gate","-j4"]"#).unwrap();
        assert_eq!(from_json, line);
    }

    #[test]
    fn command_line_refuses_an_empty_array_a_bad_program_and_bad_arguments() {
        let mut many = vec!["make".to_owned()];
        many.extend(args(65));
        let cases = [
            (vec![], "command", String::new(), ValueRule::Empty),
            (vec![String::new()], "program", String::new(), ValueRule::Empty),
            (
                vec!["./x".to_owned()],
                "program",
                "./x".to_owned(),
                ValueRule::NotBareOrAbsolute,
            ),
            (
                many.clone(),
                "argument",
                many[1..].join(" "),
                ValueRule::TooMany { max: 64 },
            ),
        ];
        for (words, what, value, rule) in cases {
            let expected = InvalidValue { what, value, rule };
            assert_eq!(CommandLine::try_from(words.clone()).unwrap_err(), expected, "{words:?}");
            let err = serde_json::from_str::<CommandLine>(&serde_json::to_string(&words).unwrap()).unwrap_err();
            assert_eq!(err.classify(), Category::Data);
        }
    }

    #[test]
    fn minutes_are_a_positive_number_and_make_a_duration() {
        for (json, secs) in [("1", 60), ("30", 1800), ("4294967295", 4_294_967_295 * 60)] {
            let minutes: Minutes = serde_json::from_str(json).unwrap();
            assert_eq!(minutes.as_duration(), Duration::from_secs(secs), "{json}");
        }
    }

    #[test]
    fn minutes_refuse_zero_negative_fractions_and_text() {
        for json in ["0", "-1", "1.5", "\"30\"", "null", "4294967296", "true"] {
            let err = serde_json::from_str::<Minutes>(json).unwrap_err();
            assert_eq!(err.classify(), Category::Data, "{json}");
        }
    }

    #[test]
    fn loopback_addr_accepts_loopback_addresses_with_a_port() {
        for text in ["127.0.0.1:8787", "[::1]:8787", "127.0.0.2:1", "127.255.255.254:65535"] {
            let addr = accepts::<LoopbackAddr>(text);
            assert_eq!(addr.as_socket_addr(), text.parse::<SocketAddr>().unwrap());
            assert_eq!(addr.to_string(), text);
        }
    }

    #[test]
    fn loopback_addr_refuses_hosts_missing_ports_other_networks_and_port_0() {
        let cases = [
            ("", ValueRule::NotSocketAddr),
            ("localhost:8787", ValueRule::NotSocketAddr),
            ("127.0.0.1", ValueRule::NotSocketAddr),
            ("127.0.0.1:99999", ValueRule::NotSocketAddr),
            ("::1:8787", ValueRule::NotSocketAddr),
            ("0.0.0.0:8787", ValueRule::NotLoopback),
            ("[::]:8787", ValueRule::NotLoopback),
            ("192.168.1.5:8787", ValueRule::NotLoopback),
            ("100.64.0.1:8787", ValueRule::NotLoopback),
            ("[fd7a:115c:a1e0::1]:8787", ValueRule::NotLoopback),
            ("127.0.0.1:0", ValueRule::ZeroPort),
            ("[::1]:0", ValueRule::ZeroPort),
        ];
        for (text, rule) in cases {
            refuses::<LoopbackAddr>("listen address", text, rule);
        }
    }

    #[test]
    fn defaults_pass_their_own_checks() {
        assert_eq!(Language::default(), accepts::<Language>("English"));
        assert_eq!(LoopbackAddr::default(), accepts::<LoopbackAddr>("127.0.0.1:8787"));
        for text in ["claude", "codex", "agy", "opencode", "agentbus"] {
            assert_eq!(Program::literal(text), accepts::<Program>(text));
        }
        for text in ["/usr/local/bin", "/usr/bin", "/bin"] {
            assert_eq!(AbsolutePath::literal(text), accepts::<AbsolutePath>(text));
        }
        for text in ["{agent} (team-agents)", "{agent}@team-agents.invalid"] {
            assert_eq!(NameTemplate::literal(text), accepts::<NameTemplate>(text));
        }
    }
}
