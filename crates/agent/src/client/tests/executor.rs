//! Negotiation with executor-aware peers and unchanged older socket protocols.

use super::*;
use agent_core::protocol::{ItemCounts, ReplyFormat, TeamSummary};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

fn summary() -> TeamSummary {
    TeamSummary {
        id: "team-20261111-193000-a1b2".parse().unwrap(),
        repo: "sample".parse().unwrap(),
        short_desc: None,
        created_at: "2026-11-11T19:30:00Z".parse().unwrap(),
        merge: agent_core::state::MergeMode::Autopilot,
        items: ItemCounts::default(),
        rounds: Vec::new(),
        splits: Vec::new(),
        part_assignments: std::collections::BTreeMap::from([(
            "round-20261111-193001-a1b2".parse().unwrap(),
            "claude-1".parse().unwrap(),
        )]),
        tier_waits: std::collections::BTreeMap::from([("1.1".parse().unwrap(), agent_core::kinds::Tier::Investigator)]),
        members: Vec::new(),
        activities: Vec::new(),
        task_plans: std::collections::BTreeMap::new(),
    }
}

async fn serve(listener: UnixListener, supported: u64) -> Vec<u64> {
    let mut versions = Vec::new();
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        while reader.read_line(&mut line).await.unwrap() != 0 {
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            let version = value["protocol"].as_u64().unwrap();
            versions.push(version);
            if version > supported {
                let refusal = Frame::Response(Response::error(
                    RequestId(0),
                    Failure {
                        kind: FailureKind::Invalid,
                        message: "unsupported protocol".into(),
                    },
                ))
                .to_line()
                .unwrap();
                write.write_all(refusal.as_bytes()).await.unwrap();
                break;
            }
            let (request, format) = Request::from_line_negotiated(&line).unwrap();
            let response = Frame::Response(Response::ok(request.id, Reply::Teams { teams: vec![summary()] }))
                .to_line_for(format)
                .unwrap();
            write.write_all(response.as_bytes()).await.unwrap();
            line.clear();
        }
        if versions.last() == Some(&supported) {
            return versions;
        }
    }
}

#[tokio::test]
async fn status_probes_executor_then_plan_split_and_legacy_and_caches_the_choice() {
    for (supported, format, expected) in [
        (6, ReplyFormat::Executor, vec![6, 6]),
        (5, ReplyFormat::Plan, vec![6, 5, 5]),
        (4, ReplyFormat::Split, vec![6, 5, 4, 4, 4]),
        (3, ReplyFormat::Legacy, vec![6, 5, 4, 3, 3]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = socket(&directory);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve(listener, supported));
        let mut connection = Connection::connect(&path).await.unwrap();
        let mut expected_summary = summary();
        if supported < 6 {
            expected_summary.part_assignments.clear();
            expected_summary.tier_waits.clear();
        }
        let expected_reply = Reply::Teams {
            teams: vec![expected_summary],
        };
        for _ in 0..2 {
            let reply = connection.request(Client::Cli, status()).await.unwrap();
            assert_eq!(reply, expected_reply);
            assert_eq!(connection.format, Some(format));
        }
        drop(connection);
        assert_eq!(server.await.unwrap(), expected);
    }
}
