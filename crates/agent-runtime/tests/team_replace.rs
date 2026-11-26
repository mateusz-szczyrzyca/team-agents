//! Process-boundary checks for the daemon's team replacement command.

use std::os::unix::fs::PermissionsExt;

use agent_core::config::{AbsolutePath, Program};
use agent_runtime::agentbus::{AgentBus, AgentBusError, BusCommand, FailureClass};

#[tokio::test]
async fn team_replace_accepts_replaced_output_and_preserves_not_found_from_the_program() {
    for (body, not_found) in [("printf 'REPLACED claude-1 claude-2\\n'", false), ("exit 3", true)] {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("agentbus");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let bus = AgentBus::new(
            program.to_str().unwrap().parse::<Program>().unwrap(),
            dir.path().to_str().unwrap().parse::<AbsolutePath>().unwrap(),
        );
        let result = bus
            .team_replace(
                &"team-20261126-084716-os01".parse().unwrap(),
                &"claude-1".parse().unwrap(),
                &"claude-2".parse().unwrap(),
            )
            .await;
        if not_found {
            assert!(matches!(
                result.unwrap_err(),
                AgentBusError::Failed {
                    command: BusCommand::TeamReplace,
                    class: FailureClass::NotFound,
                    ..
                }
            ));
        } else {
            result.unwrap();
        }
    }
}
