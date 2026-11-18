use agent_core::config::{AbsolutePath, Secrets};

use super::*;

#[test]
fn formats_secret_table_and_key_names_without_values() {
    let raw = r#"
github_token = "ghp_secret_token_12345"

[agent_env]
OPENAI_API_KEY = "sk_secret_openai_9999"
ANTHROPIC_API_KEY = "sk_ant_secret_7777"

[notification_env]
WEBHOOK_KEY = "secret_webhook_value"
"#;
    let secrets = Secrets::parse(raw, 0o600).unwrap();
    let formatted = format_secret_names(&secrets);

    assert!(!formatted.contains("ghp_secret_token_12345"));
    assert!(!formatted.contains("sk_secret_openai_9999"));
    assert!(!formatted.contains("sk_ant_secret_7777"));
    assert!(!formatted.contains("secret_webhook_value"));

    assert_eq!(
        formatted,
        "github_token\n\n[agent_env]\nANTHROPIC_API_KEY\nOPENAI_API_KEY\n\n[notification_env]\nWEBHOOK_KEY\n"
    );
}

#[tokio::test]
async fn collects_agents_toml_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);
    let sample = "[repos.example]\ngate = \"make gate\"\n";
    std::fs::write(home.config_file(), sample).unwrap();

    let collected = collect_config(&home).await.unwrap();
    assert_eq!(collected, sample);
}

#[tokio::test]
async fn reports_missing_when_agents_toml_absent() {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);

    let result = collect_config(&home).await;
    assert!(matches!(result, Err(ConfigError::Missing { .. })));
}

#[tokio::test]
async fn reports_read_failure_for_configuration_directory() {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);
    std::fs::create_dir(home.config_file()).unwrap();
    assert!(
        matches!(collect_config(&home).await, Err(ConfigError::Read { path, source: TextFileError::Read(_) }) if path == home.config_file())
    );
}

#[tokio::test]
async fn resolves_notification_directory_and_reports_invalid_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);
    assert_eq!(notification_directory(&home).await.unwrap(), home.notifications_dir());
    std::fs::write(home.config_file(), "[notifications]\n").unwrap();
    assert_eq!(notification_directory(&home).await.unwrap(), home.notifications_dir());
    let configured = dir.path().join("custom-notices");
    std::fs::write(
        home.config_file(),
        format!("[notifications]\ndirectory = {:?}\n", configured.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(notification_directory(&home).await.unwrap(), configured);
    std::fs::write(home.config_file(), "invalid TOML [").unwrap();
    assert!(matches!(
        notification_directory(&home).await,
        Err(ConfigError::Parse { path, .. }) if path == home.config_file()
    ));
    std::fs::remove_file(home.config_file()).unwrap();
    std::fs::create_dir(home.config_file()).unwrap();
    assert!(matches!(
        notification_directory(&home).await,
        Err(ConfigError::Read { path, .. }) if path == home.config_file()
    ));
}
