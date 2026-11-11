//! `secrets.toml`: the tokens and API keys the harness passes on, and the checks that keep them out of sight.
//!
//! A secret is held in a [`Secret`], whose `Debug` shows nothing and which has no `Display` or `Serialize`: the
//! only way to the text is [`Secret::expose`], at the place that hands it to a process. The errors of this module
//! never contain any text of the file, because `toml`'s own messages quote the line they refuse (SEC-2).

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;

use serde::Deserialize;

use super::values::{EnvName, ValueRule};

/// Largest secrets text accepted, in bytes.
pub const MAX_SECRETS_BYTES: usize = 1 << 16;

/// Longest single secret, in bytes: longer than any token or key in use.
const MAX_SECRET_LEN: usize = 8192;

/// The permission bits of a file that group and others may have: none, for a file of secrets.
const GROUP_AND_OTHER_BITS: u32 = 0o077;

/// The permission bits of a file's mode, without its file type.
const PERMISSION_BITS: u32 = 0o777;

/// A token, key or other secret: never shown, never serialized.
///
/// 1 to 8192 bytes without control characters, so it fits on one line. Refused, with the [`ValueRule`] it breaks:
/// the empty text, a longer one and a control character. An empty secret is refused rather than ignored: a
/// variable that is set but empty is not the same as one that is not set.
#[derive(Clone, Deserialize)]
#[serde(try_from = "String")]
pub struct Secret(String);

impl Secret {
    /// The secret's text. Call it only where the secret is handed on — to a process's environment, to a request —
    /// and never to log it, print it or put it in an error.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Why a text is not a [`Secret`]: the rule it breaks, and not the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid secret: {rule}")]
pub struct InvalidSecret {
    /// The rule the text breaks.
    pub rule: ValueRule,
}

impl TryFrom<String> for Secret {
    type Error = InvalidSecret;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let refuse = |rule| Err(InvalidSecret { rule });
        if text.is_empty() {
            return refuse(ValueRule::Empty);
        }
        if text.len() > MAX_SECRET_LEN {
            return refuse(ValueRule::TooLong { max: MAX_SECRET_LEN });
        }
        match text.chars().find(|c| c.is_control()) {
            Some(bad) => refuse(ValueRule::ForbiddenChar(bad)),
            None => Ok(Self(text)),
        }
    }
}

/// A place in a text: where `toml` found the fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The line, counted from 1.
    pub line: NonZeroUsize,
    /// The column, counted from 1, in characters.
    pub column: NonZeroUsize,
}

/// What to add to the message of [`SecretsError::Parse`] for the place of the fault, if it is known.
fn place(at: Option<Position>) -> String {
    match at {
        Some(Position { line, column }) => format!(" at line {line}, column {column}"),
        None => String::new(),
    }
}

/// Where in `text` the fault `err` is, if `toml` says and it is a place in the text.
fn position(text: &str, err: &toml::de::Error) -> Option<Position> {
    let before = text.get(..err.span()?.start)?;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let column = before.get(line_start..)?.chars().count();
    Some(Position {
        line: NonZeroUsize::MIN.saturating_add(before.matches('\n').count()),
        column: NonZeroUsize::MIN.saturating_add(column),
    })
}

/// Why a `secrets.toml` text is not valid [`Secrets`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretsError {
    /// The file can be used by its group or by others: a file of secrets must be readable by its owner only.
    #[error("the secrets file has mode {mode:04o}: only its owner may have access (chmod 600)")]
    Mode {
        /// The permission bits of the file, without its file type.
        mode: u32,
    },
    /// The text is longer than [`MAX_SECRETS_BYTES`].
    #[error("the secrets file is {len} bytes: the limit is {max}")]
    TooLarge {
        /// The length of the text, in bytes.
        len: usize,
        /// The limit, in bytes.
        max: usize,
    },
    /// The text is not valid TOML, or not the format of `secrets.toml`: an unknown key, a value of the wrong type or
    /// a secret that is refused. The text of the file is not repeated, nor what `toml` says about it, because both
    /// can quote a secret; open the file at this place.
    #[error("invalid secrets file{}", place(*.at))]
    Parse {
        /// Where the fault is, when `toml` says.
        at: Option<Position>,
    },
}

/// The validated contents of `secrets.toml`: the GitHub token and the environment variables of the processes the
/// harness starts.
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    /// The token of the GitHub account the agents act as.
    github_token: Option<Secret>,
    /// Variables for every model CLI run: `CLAUDE_CODE_OAUTH_TOKEN`, `OPENROUTER_API_KEY`, ...
    agent_env: BTreeMap<EnvName, Secret>,
    /// Variables for the notification command only.
    notification_env: BTreeMap<EnvName, Secret>,
}

/// The shape of `secrets.toml`; [`Secrets::parse`] is the only way to read one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSecrets {
    /// See [`Secrets::github_token`].
    #[serde(default)]
    github_token: Option<Secret>,
    /// The `[agent_env]` table.
    #[serde(default)]
    agent_env: BTreeMap<EnvName, Secret>,
    /// The `[notification_env]` table.
    #[serde(default)]
    notification_env: BTreeMap<EnvName, Secret>,
}

impl Secrets {
    /// Reads the text of a `secrets.toml` whose file has the permission bits `file_mode` (`st_mode`, file type
    /// bits allowed). The mode is a parameter so that no secret is read from a file others can read.
    ///
    /// # Errors
    /// [`SecretsError::Mode`] when group or others have any access; [`SecretsError::TooLarge`] for a text over
    /// [`MAX_SECRETS_BYTES`]; [`SecretsError::Parse`] for text that is not valid TOML or not the format of the file.
    /// The mode is checked first, then the size, then the text.
    pub fn parse(text: &str, file_mode: u32) -> Result<Self, SecretsError> {
        if file_mode & GROUP_AND_OTHER_BITS != 0 {
            return Err(SecretsError::Mode {
                mode: file_mode & PERMISSION_BITS,
            });
        }
        if text.len() > MAX_SECRETS_BYTES {
            return Err(SecretsError::TooLarge {
                len: text.len(),
                max: MAX_SECRETS_BYTES,
            });
        }
        // `toml`'s error is not kept: it quotes the refused line, which may hold a secret (SEC-2).
        let RawSecrets {
            github_token,
            agent_env,
            notification_env,
        } = toml::from_str(text).map_err(|err| SecretsError::Parse {
            at: position(text, &err),
        })?;
        Ok(Self {
            github_token,
            agent_env,
            notification_env,
        })
    }

    /// The token of the GitHub account the agents act as, if the file has one.
    #[must_use]
    pub fn github_token(&self) -> Option<&Secret> {
        self.github_token.as_ref()
    }

    /// The variables for every model CLI run, by name.
    #[must_use]
    pub fn agent_env(&self) -> &BTreeMap<EnvName, Secret> {
        &self.agent_env
    }

    /// The variables for the notification command only, by name.
    #[must_use]
    pub fn notification_env(&self) -> &BTreeMap<EnvName, Secret> {
        &self.notification_env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with every kind of secret; each value contains `secret` so that a test can look for it.
    const FULL: &str = r#"
github_token = "ghp_secret_0123456789"

[agent_env]
CLAUDE_CODE_OAUTH_TOKEN = "sk-ant-secret-1"
OPENROUTER_API_KEY = "or-secret-2"

[notification_env]
NTFY_TOKEN = "tk_secret_3"
"#;

    /// The mode of a file only its owner can read and write.
    const OWNER_ONLY: u32 = 0o600;

    fn env_name(name: &str) -> EnvName {
        name.parse().unwrap()
    }

    /// The faults a file can have, each with a secret-looking word on the line the fault is on, and that line.
    const FAULTS: &[(&str, usize)] = &[
        ("github_token = \"TOPSECRET\n", 1),
        ("# comment\ngithub_token = TOPSECRET\n", 2),
        ("github_token = 123456789\n", 1),
        ("github_tokn = \"TOPSECRET\"\n", 1),
        ("github_token = \"\"\n", 1),
        ("github_token = \"TOP\\u0007SECRET\"\n", 1),
        ("[agent_env]\n\"1BAD\" = \"TOPSECRET\"\n", 2),
        ("[agent_env]\nA = [\"TOPSECRET\"]\n", 2),
        ("[agent_env]\nA = \"TOPSECRET\"\nA = \"TOPSECRET\"\n", 3),
        ("[notification_env]\nA B = \"TOPSECRET\"\n", 2),
        ("github_token = \"TOPSECRET\"\ngithub_token = \"TOPSECRET\"\n", 2),
    ];

    #[test]
    fn a_file_only_its_owner_can_use_is_read() {
        for mode in [0o600, 0o400, 0o200, 0o700, 0o000, 0o100_600, 0o100_400] {
            let secrets = Secrets::parse(FULL, mode).unwrap();
            assert_eq!(
                secrets.github_token().unwrap().expose(),
                "ghp_secret_0123456789",
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_file_others_can_use_is_refused_with_its_permission_bits() {
        for (mode, shown) in [
            (0o644, 0o644),
            (0o640, 0o640),
            (0o604, 0o604),
            (0o660, 0o660),
            (0o666, 0o666),
            (0o777, 0o777),
            (0o601, 0o601),
            (0o610, 0o610),
            (0o070, 0o070),
            (0o007, 0o007),
            (0o100_644, 0o644),
            (0o100_666, 0o666),
        ] {
            let err = Secrets::parse(FULL, mode).unwrap_err();
            assert_eq!(err, SecretsError::Mode { mode: shown }, "{mode:o}");
        }
    }

    #[test]
    fn the_mode_is_checked_before_the_text_is_looked_at() {
        let err = Secrets::parse("this is not toml {{{", 0o644).unwrap_err();
        assert_eq!(err, SecretsError::Mode { mode: 0o644 });
        let err = Secrets::parse(&"#".repeat(MAX_SECRETS_BYTES + 1), 0o644).unwrap_err();
        assert_eq!(err, SecretsError::Mode { mode: 0o644 });
    }

    #[test]
    fn a_full_file_is_read_into_its_parts() {
        let secrets = Secrets::parse(FULL, OWNER_ONLY).unwrap();

        assert_eq!(secrets.github_token().unwrap().expose(), "ghp_secret_0123456789");
        let agent: Vec<_> = secrets
            .agent_env()
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose()))
            .collect();
        assert_eq!(
            agent,
            [
                ("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-secret-1"),
                ("OPENROUTER_API_KEY", "or-secret-2")
            ]
        );
        let notification: Vec<_> = secrets
            .notification_env()
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose()))
            .collect();
        assert_eq!(notification, [("NTFY_TOKEN", "tk_secret_3")]);
        assert_eq!(
            secrets
                .notification_env()
                .get(&env_name("OPENROUTER_API_KEY"))
                .map(Secret::expose),
            None
        );
    }

    #[test]
    fn an_empty_file_has_no_secrets() {
        for text in ["", "\n", "# nothing here\n"] {
            let secrets = Secrets::parse(text, OWNER_ONLY).unwrap();
            assert!(secrets.github_token().is_none());
            assert_eq!(secrets.agent_env().len(), 0);
            assert_eq!(secrets.notification_env().len(), 0);
        }
    }

    #[test]
    fn secrets_take_any_text_without_control_characters() {
        let longest = "x".repeat(8192);
        for value in ["a", "żółw 🙂", "with spaces and = signs", longest.as_str()] {
            let text = format!("github_token = \"{value}\"\n");
            let secrets = Secrets::parse(&text, OWNER_ONLY).unwrap();
            assert_eq!(secrets.github_token().unwrap().expose(), value);
        }
    }

    #[test]
    fn a_secret_refuses_the_empty_text_a_long_one_and_control_characters() {
        let long = "x".repeat(8193);
        let cases = [
            (String::new(), ValueRule::Empty),
            (long, ValueRule::TooLong { max: 8192 }),
            ("a\nb".to_owned(), ValueRule::ForbiddenChar('\n')),
            ("a\0b".to_owned(), ValueRule::ForbiddenChar('\0')),
            ("a\tb".to_owned(), ValueRule::ForbiddenChar('\t')),
        ];
        for (text, rule) in cases {
            let err = Secret::try_from(text).unwrap_err();
            assert_eq!(err, InvalidSecret { rule });
        }
    }

    #[test]
    fn debug_output_shows_no_secret() {
        let secrets = Secrets::parse(FULL, OWNER_ONLY).unwrap();
        let shown = format!("{secrets:?} {:?}", secrets.github_token());
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn a_fault_is_reported_at_its_line_and_never_with_the_text() {
        for &(text, line) in FAULTS {
            let err = Secrets::parse(text, OWNER_ONLY).unwrap_err();
            let SecretsError::Parse { at: Some(at) } = err else {
                panic!("{text:?}: {err:?}");
            };
            assert_eq!(at.line.get(), line, "{text:?}");
            let shown = format!("{err} {err:?}");
            assert!(
                !shown.contains("TOPSECRET") && !shown.contains("123456789"),
                "{text:?}: {shown}"
            );
        }
    }

    #[test]
    fn a_fault_is_reported_at_its_column_in_characters() {
        let err = Secrets::parse("żółw = 1\n", OWNER_ONLY).unwrap_err();
        let SecretsError::Parse { at: Some(at) } = err else {
            panic!("{err:?}");
        };
        assert_eq!((at.line.get(), at.column.get()), (1, 1));

        let err = Secrets::parse("# żółć\n   github_tokn = \"x\"\n", OWNER_ONLY).unwrap_err();
        let SecretsError::Parse { at: Some(at) } = err else {
            panic!("{err:?}");
        };
        assert_eq!((at.line.get(), at.column.get()), (2, 4));
    }

    #[test]
    fn the_message_of_a_parse_error_names_the_place() {
        let err = Secrets::parse("\n\ngithub_tokn = \"x\"\n", OWNER_ONLY).unwrap_err();
        assert!(err.to_string().ends_with(" at line 3, column 1"), "{err}");
    }

    #[test]
    fn a_text_over_the_limit_is_refused_and_one_at_the_limit_is_read() {
        let at_the_limit = "#".repeat(MAX_SECRETS_BYTES);
        assert!(
            Secrets::parse(&at_the_limit, OWNER_ONLY)
                .unwrap()
                .github_token()
                .is_none()
        );

        let err = Secrets::parse(&format!("{at_the_limit}#"), OWNER_ONLY).unwrap_err();
        assert_eq!(
            err,
            SecretsError::TooLarge {
                len: MAX_SECRETS_BYTES + 1,
                max: MAX_SECRETS_BYTES
            }
        );
    }
}
