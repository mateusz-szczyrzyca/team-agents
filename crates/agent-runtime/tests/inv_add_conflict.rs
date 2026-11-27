//! Conflict requests round-trip through the real agentbus CLI in a temporary home.

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::time::Duration;

    use agent_core::state::PrNumber;
    use agent_runtime::agentbus::{AgentBus, InvestigationKind, NewConflictRequest};

    #[tokio::test]
    async fn inv_add_conflict_returns_the_new_id_and_the_request_has_the_item() {
        let home = tempfile::tempdir().unwrap();
        let binary = home.path().join("agentbus");
        let mut build = tokio::process::Command::new("go");
        build
            .args(["build", "-o"])
            .arg(&binary)
            .arg(".")
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus"))
            .kill_on_drop(true);
        let compilation = tokio::time::timeout(Duration::from_secs(60), build.output())
            .await
            .unwrap()
            .unwrap();
        assert!(compilation.status.success(), "go build: {:?}", compilation.stderr);
        let bus = AgentBus::new(
            binary.to_str().unwrap().parse().unwrap(),
            home.path().to_str().unwrap().parse().unwrap(),
        );
        let request = NewConflictRequest {
            from_agent: "lead".parse().unwrap(),
            from_task: "task-20261127-075624-ab12".parse().unwrap(),
            repo: "example".parse().unwrap(),
            branch: "agents/lead/item".parse().unwrap(),
            item: "2.3".parse().unwrap(),
            question: "Resolve the conflict: café".into(),
            pr_url: "https://example.com/pull/42".into(),
            pr_number: PrNumber::new(NonZeroU32::new(42).unwrap()),
        };
        for expected in [1, 2] {
            let id = bus.inv_add_conflict(&request).await.unwrap();
            assert_eq!(id.get(), expected);
            let stored = bus.inv_get(id).await.unwrap();
            assert_eq!(stored.id, id);
            assert_eq!(stored.kind, InvestigationKind::Conflict { item: request.item });
            assert_eq!(stored.from_agent, request.from_agent);
            assert_eq!(stored.from_task, request.from_task);
            assert_eq!(stored.branch, request.branch);
            assert_eq!(stored.question, request.question);
            let state_bytes = tokio::fs::read(bus.bus_dir().join("state.json")).await.unwrap();
            let state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
            let metadata = &state["investigations"][id.to_string()];
            assert_eq!(metadata["repo"], serde_json::json!(request.repo));
            assert_eq!(metadata["pr_url"], serde_json::json!(request.pr_url));
            assert_eq!(metadata["pr_number"], serde_json::json!(request.pr_number));
            assert_eq!(
                stored.file,
                home.path()
                    .join("tasks")
                    .join(request.from_task.to_string())
                    .join("investigations")
                    .join(format!("inv-{expected}.md"))
            );
            assert!(stored.file.is_file());
        }
    }
}
