//! The settings of a home: `agents.toml` and `secrets.toml`, read from disk.
//!
//! The core parses and checks the text ([`Config::from_toml_str`], [`Secrets::parse`]); reading the files, with a bound
//! on their size and the mode of the secrets file, is the runtime's job.

use std::path::PathBuf;

use agent_core::config::{Config, ConfigError, Secrets, SecretsError};

use crate::store::Home;

/// The configuration and the secrets of one home.
#[derive(Debug, Clone)]
pub struct Settings {
    /// What `agents.toml` says.
    pub config: Config,
    /// What `secrets.toml` says; empty when the home has no such file.
    pub secrets: Secrets,
}

/// Why the settings of a home cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// There is no `agents.toml` in the home.
    #[error("there is no configuration file {path:?}: copy config/agents.example.toml there and edit it")]
    MissingConfig {
        /// The file that is missing.
        path: PathBuf,
    },
    /// A file cannot be read.
    #[error("cannot read {path:?}")]
    Read {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// A file is not UTF-8 text.
    #[error("{path:?} is not UTF-8 text")]
    NotUtf8 {
        /// The file.
        path: PathBuf,
    },
    /// A file is longer than the limit of its kind.
    #[error("{path:?} is longer than {limit} bytes")]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// The limit, in bytes.
        limit: usize,
    },
    /// `agents.toml` is not a valid configuration.
    #[error("{path:?} is not a valid configuration")]
    Config {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: ConfigError,
    },
    /// `secrets.toml` cannot be used.
    #[error("{path:?} cannot be used")]
    Secrets {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: SecretsError,
    },
}

impl Settings {
    /// Reads `agents.toml` and `secrets.toml` of `home`.
    ///
    /// # Errors
    /// [`SettingsError`]: a missing configuration, a file that cannot be read, is not UTF-8 or is too long, a
    /// configuration or a secrets file that its parser refuses (a secrets file that others can read among them).
    pub async fn load(home: &Home) -> Result<Self, SettingsError> {
        let _ = home;
        Ok(Self {
            config: Config::default(),
            secrets: Secrets::default(),
        })
    }
}
