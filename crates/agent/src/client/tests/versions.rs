//! Negotiation of version-eight status replies and fallback to older daemons that do not know the format: a new
//! client probes the reason-bearing protocol eight first and an old daemon refuses it, so the client reconnects and
//! settles for the newest format the daemon supports, which never carries the failed reason.

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
        part_assignments: std::collections::BTreeMap::new(),
        tier_waits: std::collections::BTreeMap::new(),
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
async fn status_probes_protocol_eight_then_falls_back_to_older_daemons() {
    for (supported, format, expected) in [
        (7, ReplyFormat::Versions, vec![8, 7]),
        (6, ReplyFormat::Executor, vec![8, 7, 6]),
        (5, ReplyFormat::Plan, vec![8, 7, 6, 5]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = socket(&directory);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve(listener, supported));
        let mut connection = Connection::connect(&path).await.unwrap();
        let reply = connection.request_versions(Client::Cli, status()).await.unwrap();
        assert_eq!(reply, Reply::Teams { teams: vec![summary()] });
        assert_eq!(connection.format, Some(format), "protocol {supported}");
        drop(connection);
        assert_eq!(server.await.unwrap(), expected, "protocol {supported}");
    }
}
