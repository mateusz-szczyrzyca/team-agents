//! The versions of the daemon ride on the answer to a status request; the formats older clients read do not carry them.

use super::*;
use crate::protocol::{Frame, ReplyFormat};

/// The versions of a build of the four crates, with the numbers the tests compare.
fn versions() -> Versions {
    Versions::new("0.9.1", "0.9.2", "0.9.3", "0.9.4")
}

#[test]
fn a_status_answer_carries_the_daemon_versions_and_reads_them_back() {
    let mut response = Response::ok(RequestId(7), Reply::Teams { teams: vec![] });
    response.daemon_versions = Some(versions());
    let frame = Frame::Response(response);

    let line = frame.to_line_for(ReplyFormat::Executor).unwrap();
    let read = Frame::from_line_negotiated(&line).unwrap();

    assert_eq!(read, frame);
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        value.pointer("/daemon_versions"),
        Some(&serde_json::json!({
            "agent": "0.9.1",
            "agent_core": "0.9.2",
            "agent_runtime": "0.9.3",
            "agent_web": "0.9.4",
        })),
        "{line}"
    );
}

#[test]
fn an_answer_without_versions_is_written_as_before() {
    let frame = Frame::Response(Response::ok(RequestId(8), Reply::Teams { teams: vec![] }));

    let line = frame.to_line_for(ReplyFormat::Executor).unwrap();

    assert!(!line.contains("daemon_versions"), "{line}");
    assert_eq!(Frame::from_line_negotiated(&line).unwrap(), frame);
}

#[test]
fn the_formats_older_clients_read_leave_the_versions_out() {
    let mut response = Response::ok(RequestId(9), Reply::Teams { teams: vec![] });
    response.daemon_versions = Some(versions());
    let frame = Frame::Response(response);

    for format in [ReplyFormat::Legacy, ReplyFormat::Split, ReplyFormat::Plan] {
        let line = frame.to_line_for(format).unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value.pointer("/daemon_versions"), None, "{format:?}: {line}");
    }
    let executor = frame.to_line_for(ReplyFormat::Executor).unwrap();
    let value: serde_json::Value = serde_json::from_str(&executor).unwrap();
    assert!(value.pointer("/daemon_versions").is_some(), "{executor}");
}

#[test]
fn a_daemon_that_sends_no_versions_reads_as_none() {
    let line =
        r#"{"type":"response","protocol":6,"id":3,"result":{"status":"ok","reply":{"kind":"teams","teams":[]}}}"#;

    let Frame::Response(response) = Frame::from_line_negotiated(line).unwrap() else {
        panic!("the line is an answer");
    };

    assert_eq!(response.daemon_versions, None);
}

#[test]
fn the_versions_name_each_crate_and_refuse_unknown_fields() {
    let line = r#"{"agent":"1","agent_core":"2","agent_runtime":"3","agent_web":"4","agent_cli":"5"}"#;

    assert!(serde_json::from_str::<Versions>(line).is_err());
    assert_eq!(serde_json::to_value(versions()).unwrap()["agent_web"], "0.9.4");
}
