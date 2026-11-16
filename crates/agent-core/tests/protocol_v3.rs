//! Version three refuses older peers before attempting to decode their payloads.

use agent_core::protocol::{Frame, ProtocolError, Request, UnsupportedVersion};

#[test]
fn version_two_requests_and_frames_are_refused_even_when_their_payload_is_valid() {
    for text in [
        include_str!("fixtures/protocol-v2-requests.jsonl"),
        include_str!("fixtures/protocol-v2-round-requests.jsonl"),
    ] {
        for line in text.lines() {
            assert!(matches!(
                Request::from_line(line),
                Err(ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 2 }))
            ));
        }
    }
    for text in [
        include_str!("fixtures/protocol-v2-frames.jsonl"),
        include_str!("fixtures/protocol-v2-round-frames.jsonl"),
    ] {
        for line in text.lines() {
            assert!(matches!(
                Frame::from_line(line),
                Err(ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 2 }))
            ));
        }
    }
}
