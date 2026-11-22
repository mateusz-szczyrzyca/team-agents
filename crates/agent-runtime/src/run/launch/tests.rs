use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, Secrets};

use super::check_environment;
use crate::git::CommitIdentity;

#[test]
fn check_environment_has_the_agents_secrets_path_and_home_but_no_daemon_secret() {
    let secrets = Secrets::parse(
        "github_token = \"ghp_example\"\n[agent_env]\nAGENT_KEY = \"agent-value\"\n[notification_env]\nNOTIFY_KEY = \"notify-value\"\n",
        0o600,
    )
    .expect("secrets");
    let identity = CommitIdentity::new("Agent".to_owned(), "agent@example.com".to_owned(), None);
    let binary_dir = AbsolutePath::from_str("/opt/agent/bin").expect("binary dir");

    let env = check_environment(&secrets, &binary_dir, &[], Path::new("/home/h/.agents"), &identity)
        .expect("environment");
    let map: BTreeMap<String, String> = env.into_iter().collect();

    assert_eq!(map.get("AGENT_KEY").map(String::as_str), Some("agent-value"));
    assert_eq!(
        map.get("PATH").map(|path| path.split(':').next()),
        Some(Some("/opt/agent/bin"))
    );
    assert_eq!(map.get("HOME").map(String::as_str), Some("/home/h/.agents"));
    assert!(!map.contains_key("NOTIFY_KEY"), "a notification secret is not given to a check");
}
