//! Command-level coverage of executor-aware status negotiation and older daemon fallback.

use super::{Connection, status};
use agent_core::protocol::{Failure, FailureKind, Frame, Reply, Request, RequestId, Response, TeamSummary};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

fn summary() -> TeamSummary {
    let mut wire = serde_json::to_value(TeamSummary::from(&crate::fixtures::team_state().teams()[0])).unwrap();
    wire["splits"] = serde_json::json!([{"parent":"1.1","approved_by":"harness","parts":[
        {"title":"First","delivers":"Guide","checks":"Read","depends_on":[],"tier":"normal"},
        {"title":"Second","delivers":"Guide","checks":"Read","depends_on":["a"],"tier":"investigator"}
    ]}]);
    wire["rounds"] = serde_json::json!([
        {"id":"round-20261111-153001-q1w2","item":"1.1","opened_at":crate::fixtures::at(1),
         "state":{"status":"closed","at":crate::fixtures::at(2),"how":"merged"},
         "part":{"status":"merged","key":"a","pr":crate::fixtures::pr(7)}}
    ]);
    wire["part_assignments"] = serde_json::json!({"round-20261111-153001-q1w2":"claude-1"});
    wire["tier_waits"] = serde_json::json!({"1.1":"investigator"});
    serde_json::from_value(wire).unwrap()
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
        if versions.last().is_some_and(|version| *version <= supported) {
            return versions;
        }
    }
}

#[tokio::test]
async fn status_command_negotiates_executor_metadata_and_falls_back_to_plan_daemons() {
    for (supported, expected_versions, expected_lines) in [
        (
            6,
            vec![6],
            vec![
                "    1.1a  done  First",
                "    1.1b  waiting for investigator agent  Second",
                "      Split 1.1a (item 1.1, part a); approved by harness; requested normal; assigned claude-1; other parts: 1.1b (not published).",
                "      Split 1.1b (item 1.1, part b); approved by harness; requested investigator; assigned not assigned; other parts: 1.1a (#7).",
            ],
        ),
        (
            5,
            vec![6, 5],
            vec![
                "    1.1a  done  First",
                "    1.1b  to do  Second",
                "      Split 1.1a (item 1.1, part a); approved by harness; requested normal; assigned not recorded; other parts: 1.1b (not published).",
                "      Split 1.1b (item 1.1, part b); approved by harness; requested investigator; assigned not assigned; other parts: 1.1a (#7).",
            ],
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve(listener, supported));
        let mut connection = Connection::connect(&path).await.unwrap();
        let mut output = Vec::new();
        status(&mut connection, None, &mut output).await.unwrap();
        drop(connection);
        assert_eq!(server.await.unwrap(), expected_versions);
        let output = String::from_utf8(output).unwrap();
        let lines: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("    1.1") || line.starts_with("      Split "))
            .collect();
        assert_eq!(lines, expected_lines, "protocol {supported}");
    }
}
