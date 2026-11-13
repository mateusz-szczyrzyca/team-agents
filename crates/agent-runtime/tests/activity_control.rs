//! What the activity log of a task shows of a control of the human: what was done and by which client.

#[cfg(test)]
mod activity_control {
    use std::str::FromStr;

    use agent_core::cli::ActivityEvent;
    use agent_core::config::AbsolutePath;
    use agent_core::ids::TaskId;
    use agent_core::protocol::{Client, ControlAction};
    use agent_core::state::Timestamp;
    use agent_runtime::store::{Home, StateStore};
    use serde_json::{Value, json};

    const TASK: &str = "task-20261111-153601-k3x9";

    fn home_in(dir: &tempfile::TempDir) -> Home {
        Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap())
    }

    /// Appends `event` to the log of the task at 5 seconds and returns the lines of the log.
    async fn logged(event: ActivityEvent) -> Vec<Value> {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        let task = TaskId::from_str(TASK).unwrap();
        let at = Timestamp::from_str("2026-11-11T15:36:05Z").unwrap();

        store.append_activity(&task, at, &event).await.unwrap();

        std::fs::read_to_string(home.activity_log(&task))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn a_control_of_the_command_line_is_logged_with_its_action_and_its_client() {
        let lines = logged(ActivityEvent::Control {
            action: ControlAction::Pause,
            by: Client::Cli,
        })
        .await;

        assert_eq!(
            lines,
            [json!({"at": "2026-11-11T15:36:05Z", "kind": "control", "action": "pause", "by": {"kind": "cli"}})]
        );
    }

    #[tokio::test]
    async fn a_control_of_the_dashboard_names_the_login() {
        let lines = logged(ActivityEvent::Control {
            action: ControlAction::Say,
            by: Client::Dashboard {
                login: "you@example.com".parse().unwrap(),
            },
        })
        .await;

        assert_eq!(
            lines,
            [json!({"at": "2026-11-11T15:36:05Z", "kind": "control", "action": "say",
                    "by": {"kind": "dashboard", "login": "you@example.com"}})]
        );
    }
}
