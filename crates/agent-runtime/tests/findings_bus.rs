//! Findings command arguments, typed JSON and failure propagation.
#[cfg(test)]
mod tests {
    use agent_core::config::{AbsolutePath, Program};
    use agent_runtime::agentbus::{AgentBus, AgentBusError, BusCommand};
    use std::os::unix::fs::PermissionsExt;

    fn bus(script: &str) -> (tempfile::TempDir, AgentBus) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bus");
        std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let bus = AgentBus::new(
            bin.to_str().unwrap().parse::<Program>().unwrap(),
            dir.path().to_str().unwrap().parse::<AbsolutePath>().unwrap(),
        );
        (dir, bus)
    }

    #[tokio::test]
    async fn findings_command_reads_open_typed_records_and_passes_repo() {
        let (dir, bus) = bus(r#"printf '%s\n' "$@" > "$AGENTS_HOME/args"
printf '%s' '[{"id":"F-8","ts":"2026-11-27T00:00:00Z","author":"other","task":"task-20261127-112701-abcd","kind":"trap","paths":["src/lib.rs"],"text":"lesson"}]'"#);
        let findings = bus.findings(&"example".parse().unwrap()).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id.to_string(), "F-8");
        assert_eq!(findings[0].text, "lesson");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("args")).unwrap(),
            format!(
                "findings\n--dir\n{}\n--repo\nexample\n--json\n",
                dir.path().join("bus").display()
            )
        );
    }

    #[tokio::test]
    async fn findings_rejects_invalid_json() {
        let (_dir, bus) = bus("printf broken");
        assert!(matches!(
            bus.findings(&"example".parse().unwrap()).await,
            Err(AgentBusError::Decode {
                command: BusCommand::Findings,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn findings_propagates_bus_failure() {
        let (_dir, bus) = bus("exit 1");
        assert!(matches!(
            bus.findings(&"example".parse().unwrap()).await,
            Err(AgentBusError::Failed {
                command: BusCommand::Findings,
                ..
            })
        ));
    }
}
