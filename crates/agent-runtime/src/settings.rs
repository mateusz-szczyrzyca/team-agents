//! The settings of a home: `agents.toml` and `secrets.toml`, read from disk.
//!
//! The core parses and checks the text ([`Config::from_toml_str`], [`Secrets::parse`]); reading the files, with a bound
//! on their size and the mode of the secrets file, is the runtime's job.

use std::path::{Path, PathBuf};

use agent_core::config::{Config, ConfigError, MAX_CONFIG_BYTES, MAX_SECRETS_BYTES, Secrets, SecretsError};

use crate::store::Home;
use crate::text_file::{TextFile, TextFileError, read_text_file};

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
        /// Where the text stops being UTF-8.
        source: std::string::FromUtf8Error,
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
        let config_path = home.config_file();
        let config = read_text(&config_path, MAX_CONFIG_BYTES)
            .await?
            .ok_or_else(|| SettingsError::MissingConfig {
                path: config_path.clone(),
            })?;
        let config = Config::from_toml_str(&config.text).map_err(|source| SettingsError::Config {
            path: config_path,
            source,
        })?;
        let secrets_path = home.secrets_file();
        let secrets = match read_text(&secrets_path, MAX_SECRETS_BYTES).await? {
            Some(file) => Secrets::parse(&file.text, file.mode).map_err(|source| SettingsError::Secrets {
                path: secrets_path,
                source,
            })?,
            // No secrets file is the specified state of a home that needs none, such as a trial with fake CLIs.
            None => Secrets::default(),
        };
        Ok(Self { config, secrets })
    }
}

/// Reads the file at `path`, at most `limit` bytes of it; `None` when there is no such file.
async fn read_text(path: &Path, limit: usize) -> Result<Option<TextFile>, SettingsError> {
    read_text_file(path, limit).await.map_err(|error| match error {
        TextFileError::Read(source) => SettingsError::Read {
            path: path.to_path_buf(),
            source,
        },
        TextFileError::TooLarge { limit } => SettingsError::TooLarge {
            path: path.to_path_buf(),
            limit,
        },
        TextFileError::NotUtf8(source) => SettingsError::NotUtf8 {
            path: path.to_path_buf(),
            source,
        },
    })
}
