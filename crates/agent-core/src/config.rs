//! The configuration: the typed model of `agents.toml` and `secrets.toml` (see `docs/configuration.md`).

mod values;

pub use values::{
    AbsolutePath, CliArgs, CommandLine, EnvName, GithubLogin, InvalidValue, Language, LoopbackAddr, Minutes, ModelName,
    NameTemplate, Program, RepoName, TailscaleLogin, ValueRule,
};
