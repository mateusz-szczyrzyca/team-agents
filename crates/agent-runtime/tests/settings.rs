//! Reading `agents.toml` and `secrets.toml` of a home: what loads, and every way a file can be unusable.

#[cfg(test)]
mod settings_files {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::{AbsolutePath, ConfigError, MAX_CONFIG_BYTES, MAX_SECRETS_BYTES, Secret, SecretsError};
    use agent_runtime::settings::{Settings, SettingsError};
    use agent_runtime::store::Home;

    /// A configuration with one agent, one investigator and a repository.
    const CONFIG: &str = r#"
    [agents.claude-1]
    cli = "claude"

    [investigators.inv-opus]
    cli = "claude"
    model = "opus"

    [tiers]
    normal = ["claude-1"]

    [repos.team-agents]
    base_branch = "main"
    gate = ["make", "gate"]
    "#;

    fn home_in(dir: &tempfile::TempDir) -> Home {
        Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap())
    }

    fn write(path: &Path, text: &str, mode: u32) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[tokio::test]
    async fn a_home_with_both_files_loads_them() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), CONFIG, 0o644);
        write(
            &home.secrets_file(),
            "github_token = \"ghp_example\"\n[agent_env]\nOPENROUTER_API_KEY = \"or-1\"\n",
            0o600,
        );

        let settings = Settings::load(&home).await.unwrap();

        assert_eq!(settings.config.normal_agents().len(), 1);
        assert_eq!(settings.config.investigators().len(), 1);
        assert_eq!(settings.secrets.github_token().map(Secret::expose), Some("ghp_example"));
        assert_eq!(settings.secrets.agent_env().len(), 1);
    }

    #[tokio::test]
    async fn a_home_without_a_secrets_file_has_no_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), CONFIG, 0o644);

        let settings = Settings::load(&home).await.unwrap();

        assert!(settings.secrets.github_token().is_none());
        assert!(settings.secrets.agent_env().is_empty());
    }

    #[tokio::test]
    async fn a_home_without_a_configuration_says_which_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);

        let error = Settings::load(&home).await.unwrap_err();

        assert!(matches!(&error, SettingsError::MissingConfig { path } if *path == home.config_file()));
    }

    #[tokio::test]
    async fn a_configuration_the_core_refuses_is_reported_with_its_file_and_cause() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), "[agents.claude-1]\ncli = \"claude\"\n", 0o644);

        let error = Settings::load(&home).await.unwrap_err();

        assert!(
            matches!(
                &error,
                SettingsError::Config { path, source: ConfigError::AgentInNoList { .. } } if *path == home.config_file()
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_secrets_file_that_others_can_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), CONFIG, 0o644);
        write(&home.secrets_file(), "github_token = \"ghp_example\"\n", 0o640);

        let error = Settings::load(&home).await.unwrap_err();

        assert!(
            matches!(
                &error,
                SettingsError::Secrets { path, source: SecretsError::Mode { mode: 0o640 } } if *path == home.secrets_file()
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_secrets_file_that_does_not_parse_is_refused_without_showing_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), CONFIG, 0o644);
        write(&home.secrets_file(), "github_token = ghp_unquoted_secret\n", 0o600);

        let error = Settings::load(&home).await.unwrap_err();

        assert!(
            matches!(
                &error,
                SettingsError::Secrets {
                    source: SecretsError::Parse { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        assert!(!format!("{error:?}").contains("ghp_unquoted_secret"));
    }

    #[tokio::test]
    async fn a_configuration_over_the_limit_is_refused_by_size() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), &"#".repeat(MAX_CONFIG_BYTES + 1), 0o644);

        let error = Settings::load(&home).await.unwrap_err();

        assert!(
            matches!(&error, SettingsError::TooLarge { path, limit } if *path == home.config_file() && *limit == MAX_CONFIG_BYTES),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_secrets_file_over_the_limit_is_refused_by_size() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        write(&home.config_file(), CONFIG, 0o644);
        write(&home.secrets_file(), &"#".repeat(MAX_SECRETS_BYTES + 1), 0o600);

        let error = Settings::load(&home).await.unwrap_err();

        assert!(
            matches!(&error, SettingsError::TooLarge { path, limit } if *path == home.secrets_file() && *limit == MAX_SECRETS_BYTES),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_file_that_is_not_utf8_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        std::fs::write(home.config_file(), [0xff, 0xfe, 0x00]).unwrap();

        let error = Settings::load(&home).await.unwrap_err();

        assert!(matches!(&error, SettingsError::NotUtf8 { path, .. } if *path == home.config_file()));
    }

    #[tokio::test]
    async fn a_configuration_that_is_a_directory_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        std::fs::create_dir(home.config_file()).unwrap();

        let error = Settings::load(&home).await.unwrap_err();

        assert!(matches!(&error, SettingsError::Read { path, .. } if *path == home.config_file()));
    }
}
