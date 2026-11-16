//! Publication wait diagnostics remain visible in the default CLI format.

#[test]
fn publication_wait_text_shows_reason_branch_tip_and_optional_gate_head() {
    use agent_core::history::{Cut, MaxTextBytes};
    use agent_core::log::{LogEvent, LogRecord};
    use agent_runtime::history::{HistoryDb, Store};

    for gate_head in [None, Some("1111111111111111111111111111111111111111")] {
        let dir = tempfile::tempdir().unwrap();
        let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).unwrap();
        let reason = if gate_head.is_some() {
            "gate_at_another_commit"
        } else {
            "no_gate_record"
        };
        let branch = "agents/claude-1/finished";
        let tip = "2222222222222222222222222222222222222222";
        let mut fields = std::collections::BTreeMap::from([
            ("branch".to_owned(), serde_json::json!(branch)),
            ("reason".to_owned(), serde_json::json!(reason)),
            ("tip".to_owned(), serde_json::json!(tip)),
        ]);
        if let Some(head) = gate_head {
            fields.insert("gate_head".to_owned(), serde_json::json!(head));
        }
        db.write_logs(&[LogRecord {
            at: "2026-11-15T00:00:00Z".parse().unwrap(),
            module: "test".to_owned(),
            event: LogEvent::PublicationWaits,
            team: None,
            task: None,
            message: "a finished branch is not published yet".to_owned(),
            fields,
            cut: Cut::default(),
        }])
        .unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent"))
            .env("AGENTS_HOME", dir.path())
            .args(["log", "--event", "publication_waits"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
        let text = String::from_utf8(output.stdout).unwrap();
        let suffix = gate_head.map_or_else(String::new, |head| format!(" gate_head={head}"));
        assert_eq!(
            text,
            format!(
                "2026-11-15T00:00:00Z warn publication_waits a finished branch is not published yet branch={branch} reason={reason} tip={tip}{suffix}\n"
            )
        );
    }
}
