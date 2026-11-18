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
