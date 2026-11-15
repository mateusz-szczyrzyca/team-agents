//! Following is ordered and durable across writer restart and retention.
#[cfg(test)]
mod tests {
    use agent_core::history::{Cut, MaxTextBytes};
    use agent_core::log::{LogEvent, LogRecord};
    use agent_runtime::history::{HistoryDb, Retention, Store};
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn follows_new_rows_once_in_order_across_restart_and_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let log = LogRecord {
            at: "2026-11-15T12:00:00Z".parse().unwrap(),
            module: "follow".to_owned(),
            event: LogEvent::DaemonStarted,
            team: None,
            task: None,
            message: "first".to_owned(),
            fields: std::collections::BTreeMap::new(),
            cut: Cut::default(),
        };
        let mut db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
        db.write_logs(std::slice::from_ref(&log)).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent"))
            .env("AGENTS_HOME", dir.path())
            .args(["log", "--json", "-f"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                tx.send(line).unwrap();
            }
        });
        let first = rx.recv_timeout(Duration::from_secs(10));
        if first.is_err() {
            let output = child.wait_with_output().unwrap();
            reader.join().unwrap();
            panic!(
                "follower did not print initial row: {:?}, stderr={:?}",
                first,
                String::from_utf8(output.stderr).unwrap()
            );
        }
        let first: serde_json::Value = serde_json::from_str(&first.unwrap().unwrap()).unwrap();
        assert_eq!(first["message"], "first");
        db.write_logs(&[LogRecord {
            message: "second".to_owned(),
            ..log.clone()
        }])
        .unwrap();
        let second: serde_json::Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()).unwrap();
        assert_eq!(second["message"], "second");
        db.retain(&Retention {
            closed_before: None,
            logs_before: Some("2026-11-16T12:00:00Z".parse().unwrap()),
            home: dir.path().to_path_buf(),
        })
        .unwrap();
        drop(db);
        let mut db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
        db.write_logs(&[LogRecord {
            message: "third".to_owned(),
            ..log
        }])
        .unwrap();
        let third: serde_json::Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()).unwrap();
        assert_eq!(third["message"], "third");
        assert!(first["id"].as_i64().unwrap() < second["id"].as_i64().unwrap());
        assert!(second["id"].as_i64().unwrap() < third["id"].as_i64().unwrap());
        child.kill().unwrap();
        child.wait().unwrap();
        reader.join().unwrap();
        assert!(rx.try_iter().collect::<Vec<_>>().is_empty());
    }
}
