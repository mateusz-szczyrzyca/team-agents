//! Protocol compatibility for `daemon_versions`: stripped for protocol-six clients and carried in protocol seven.

use agent_core::protocol::{
    Client, Command, Frame, Outcome, Reply, ReplyFormat, Request, RequestId, Response, StatusRequest,
    VERSIONS_PROTOCOL_VERSION, Versions,
};
use serde::{Deserialize, Serialize};

/// The strict response shape understood by protocol-six clients before `daemon_versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OldV6Response {
    /// The protocol version tag on the wire (`6`).
    protocol: u32,
    /// The request identifier.
    id: RequestId,
    /// The request outcome.
    result: Outcome,
    /// Optional split acknowledgement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split: Option<String>,
}

/// The strict frame wrapper understood by protocol-six clients before `daemon_versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OldV6Frame {
    /// A response frame without `daemon_versions`.
    Response(OldV6Response),
}

fn sample_versions() -> Versions {
    Versions {
        agent: String::from("0.58.0"),
        agent_core: String::from("0.58.0"),
        agent_runtime: String::from("0.58.0"),
        agent_web: String::from("0.58.0"),
    }
}

#[test]
fn protocol_six_strips_daemon_versions_so_strict_old_v6_clients_decode_status_replies() {
    let mut response = Response::ok(RequestId(1), Reply::Teams { teams: Vec::new() });
    response.daemon_versions = Some(sample_versions());
    let frame = Frame::Response(response);

    let wire = frame.to_line_for(ReplyFormat::Executor).unwrap();
    let decoded: OldV6Frame = serde_json::from_str(&wire).expect("strict v6 response decodes without daemon_versions");

    assert_eq!(
        decoded,
        OldV6Frame::Response(OldV6Response {
            protocol: 6,
            id: RequestId(1),
            result: Outcome::Ok {
                reply: Reply::Teams { teams: Vec::new() },
            },
            split: None,
        })
    );
}

#[test]
fn protocol_seven_negotiates_and_round_trips_daemon_versions() {
    let request = Request::new(
        RequestId(2),
        Client::Cli,
        Command::Status(StatusRequest { target: None }),
    );
    let request_wire = request.to_line_for(ReplyFormat::Versions).unwrap();
    let request_json: serde_json::Value = serde_json::from_str(&request_wire).unwrap();
    assert_eq!(request_json["protocol"], VERSIONS_PROTOCOL_VERSION);
    assert_eq!(
        Request::from_line_negotiated(&request_wire).unwrap(),
        (request, ReplyFormat::Versions)
    );

    let mut response = Response::ok(RequestId(2), Reply::Teams { teams: Vec::new() });
    response.daemon_versions = Some(sample_versions());
    let frame = Frame::Response(response);

    let frame_wire = frame.to_line_for(ReplyFormat::Versions).unwrap();
    let frame_json: serde_json::Value = serde_json::from_str(&frame_wire).unwrap();
    assert_eq!(frame_json["protocol"], VERSIONS_PROTOCOL_VERSION);
    assert_eq!(
        frame_json
            .pointer("/daemon_versions/agent")
            .and_then(serde_json::Value::as_str),
        Some("0.58.0")
    );
    assert_eq!(Frame::from_line_negotiated(&frame_wire).unwrap(), frame);
}
