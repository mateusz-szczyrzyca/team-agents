//! `agent debug` lists the accounts of the configuration with the names of their variables and never their values.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agent_core::config::AbsolutePath;
use agent_core::state::{Crew, Member, MergeMode, Participant, State, TaskListText, Team};

use super::{DebugArgs, DebugTarget, execute_debug};
use crate::store::Home;

/// A secret of an account that must never show in the bundle.
const MARKER: &str = "MARKER-7f3a";

/// The configuration: two accounts of two owners, with agents and an investigator of the first.
const CONFIG: &str = r#"
[accounts.comp-claude]
owner = "comp"
[accounts.comp-claude.env]
CLAUDE_CONFIG_DIR = "/home/x/.claude-comp"

[accounts.mateusz-codex]
owner = "mateusz"

[agents.comp-claude-1]
cli = "claude"
account = "comp-claude"

[agents.mateusz-codex-1]
cli = "codex"
account = "mateusz-codex"

[investigators.comp-inv-opus]
cli = "claude"
model = "opus"
account = "comp-claude"

[tiers]
normal = ["comp-claude-1", "mateusz-codex-1"]
"#;

/// A home with a team of one task, the configuration above and a secret of the first account.
fn home_with_accounts() -> (tempfile::TempDir, Home, DebugArgs) {
    let dir = tempfile::tempdir().unwrap();
    let absolute = AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&absolute);
    let team = "team-20261117-120000-a1b2".parse().unwrap();
    let member = Member::new(
        "task-20261117-120000-a1b2".parse().unwrap(),
        Participant::Normal("comp-claude-1".parse().unwrap()),
    );
    let mut state = State::new();
    state
        .insert_team(Team::new(
            team,
            "example".parse().unwrap(),
            "2026-11-17T12:00:00Z".parse().unwrap(),
            MergeMode::Autopilot,
            TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Work\n".to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        ))
        .unwrap();
    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    std::fs::write(home.config_file(), CONFIG).unwrap();
    std::fs::write(
        home.secrets_file(),
        format!("[account_env.comp-claude]\nCLAUDE_CODE_OAUTH_TOKEN = \"{MARKER}\"\n"),
    )
    .unwrap();
    std::fs::set_permissions(home.secrets_file(), std::fs::Permissions::from_mode(0o600)).unwrap();
    let args = DebugArgs {
        target: DebugTarget::Team("team-20261117-120000-a1b2".parse().unwrap()),
        out: None,
        stale_minutes: None,
        daemon_log: None,
    };
    (dir, home, args)
}

/// Every file under `dir`, with its path, so that none can hold what must not be shown.
fn files_under(dir: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            let bytes = std::fs::read(&path).unwrap();
            found.push((path, bytes));
        }
    }
    found
}

#[tokio::test]
async fn debug_lists_accounts_without_values() {
    let (_dir, home, args) = home_with_accounts();

    let result = execute_debug(&home, args, None, &mut Vec::new()).await.unwrap();

    let summary = std::fs::read_to_string(&result.summary_path).unwrap();
    let section = summary
        .split("## Accounts\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert_eq!(
        section,
        "
- `comp-claude` (owner `comp`)
  - Agents: comp-claude-1, comp-inv-opus
  - Environment variables: CLAUDE_CONFIG_DIR
  - Secret variables: CLAUDE_CODE_OAUTH_TOKEN
- `mateusz-codex` (owner `mateusz`)
  - Agents: mateusz-codex-1
  - Environment variables: none
  - Secret variables: none
"
    );
    for (path, bytes) in files_under(&result.bundle_dir) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(MARKER), "{} holds the secret", path.display());
    }
}

#[tokio::test]
async fn debug_says_the_accounts_cannot_be_read_when_the_configuration_does_not_load() {
    let (_dir, home, args) = home_with_accounts();
    std::fs::write(home.config_file(), "[accounts.comp-claude]\n").unwrap();

    let result = execute_debug(&home, args, None, &mut Vec::new()).await.unwrap();

    let summary = std::fs::read_to_string(&result.summary_path).unwrap();
    let section = summary
        .split("## Accounts\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert!(section.contains("Could not read accounts: "), "{section}");
    assert!(!section.contains("No accounts are configured."), "{section}");
}

#[tokio::test]
async fn debug_says_the_accounts_cannot_be_read_when_the_configuration_file_is_missing() {
    let (_dir, home, args) = home_with_accounts();
    std::fs::remove_file(home.config_file()).unwrap();

    let result = execute_debug(&home, args, None, &mut Vec::new()).await.unwrap();

    let summary = std::fs::read_to_string(&result.summary_path).unwrap();
    let section = summary
        .split("## Accounts\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert!(section.contains("Could not read accounts: "), "{section}");
    assert!(!section.contains("No accounts are configured."), "{section}");
}
