//! The control protocol: what the CLI and the dashboard say to the daemon, and what it says back, as JSON lines over
//! its Unix socket (see `docs/state.md`).
//!
//! A client sends one [`Request`] per line; the daemon answers each with a [`Response`] and, to a client that
//! subscribed, sends [`Event`]s as the state changes — a [`Frame`] is either. Every line carries `"protocol": 1`
//! ([`PROTOCOL_VERSION`]); a line of another version is refused with [`ProtocolError::UnsupportedVersion`] before
//! anything else of it is read, so a client and a daemon of different versions say so and do not misread each other.
//! The types here only describe lines and read and write them; the socket is the daemon's.

mod event;
mod reply;
mod request;

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use event::{ControlAction, Event, EventKind};
pub use reply::{Failure, FailureKind, ItemCounts, MemberSummary, Outcome, Reply, Response, TeamSummary};
pub use request::{
    AgentList, AgentListError, AnswerInput, AnswerList, AnswerListError, AnswerRequest, Client, Command,
    ControlRequest, GateRequest, MAX_ANSWERS, MAX_RUN_AGENTS, MAX_SAY_BYTES, Request, RequestId, RunRequest, SayError,
    SayRequest, SayText, ShowRequest, StatusRequest, SubscribeRequest, Target,
};

/// The protocol version that this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// Longest line a client may send, in bytes: a task list (1 MiB at most) with room for the rest of the request.
pub const MAX_REQUEST_BYTES: usize = 2 << 20;

/// Longest line the daemon may send, in bytes: a reply that lists teams in full can be large.
pub const MAX_FRAME_BYTES: usize = 32 << 20;

/// The protocol version of a line: written as the number [`PROTOCOL_VERSION`], and only that number is read.
///
/// A type with one value, so that a message of another version cannot be mistaken for one of this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersion;

version_type!(ProtocolVersion, PROTOCOL_VERSION, UnsupportedVersion);

/// A line of a protocol version this build does not speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the line has protocol version {found}; this build speaks version {PROTOCOL_VERSION}")]
pub struct UnsupportedVersion {
    /// The version the line declares.
    pub found: u64,
}

/// Why a line could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The line is longer than the limit for its direction.
    #[error("the line has {len} bytes; the limit is {max}")]
    TooLong {
        /// The line's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The line declares a protocol version this build does not speak.
    #[error(transparent)]
    UnsupportedVersion(#[from] UnsupportedVersion),
    /// The line is not a message: not JSON, a field missing or unknown, or a value that is not valid.
    #[error("the line is not a valid message")]
    Malformed {
        /// The parser's reason, with the column.
        source: serde_json::Error,
    },
}

/// A message could not be written as a line.
#[derive(Debug, thiserror::Error)]
#[error("the message cannot be written as JSON")]
pub struct EncodeError {
    /// What the serializer said.
    #[source]
    source: serde_json::Error,
}

/// A line from the daemon: the answer to a request, or an event of the state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    /// The answer to a request.
    Response(Response),
    /// A change of the state, sent to a client that subscribed.
    Event(Event),
}

/// Only the protocol version of a line, read before the rest.
#[derive(Deserialize)]
struct VersionProbe {
    /// The version the line declares.
    protocol: u64,
}

/// Writes `message` as one line ending with a newline; the JSON of a line never contains a newline itself.
fn encode<T: Serialize>(message: &T) -> Result<String, EncodeError> {
    let mut line = serde_json::to_string(message).map_err(|source| EncodeError { source })?;
    line.push('\n');
    Ok(line)
}

/// Reads a line of at most `max` bytes, checking its protocol version before the rest.
fn decode<T: DeserializeOwned>(line: &str, max: usize) -> Result<T, ProtocolError> {
    if line.len() > max {
        return Err(ProtocolError::TooLong { len: line.len(), max });
    }
    let probe: VersionProbe = serde_json::from_str(line).map_err(|source| ProtocolError::Malformed { source })?;
    if probe.protocol != u64::from(PROTOCOL_VERSION) {
        return Err(UnsupportedVersion { found: probe.protocol }.into());
    }
    serde_json::from_str(line).map_err(|source| ProtocolError::Malformed { source })
}

impl Frame {
    /// The frame as one line ending with a newline.
    ///
    /// # Errors
    /// [`EncodeError`] when the serializer fails.
    pub fn to_line(&self) -> Result<String, EncodeError> {
        encode(self)
    }

    /// Reads a line the daemon sent.
    ///
    /// # Errors
    /// [`ProtocolError`]: the line is over [`MAX_FRAME_BYTES`], declares another protocol version (checked first), or
    /// is not a valid frame.
    pub fn from_line(line: &str) -> Result<Self, ProtocolError> {
        decode(line, MAX_FRAME_BYTES)
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PROTOCOL_VERSION}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::protocol::{EventKind, Reply};
    use crate::state::fixtures::{at, team_id};

    /// A request line with the given body after the version, id and client.
    fn request_line(body: &str) -> String {
        format!("{{\"protocol\":1,\"id\":7,\"client\":{{\"kind\":\"cli\"}},\"request\":{body}}}")
    }

    /// The protocol version of every message of this build.
    #[test]
    fn the_protocol_version_is_one_and_is_written_as_a_number() {
        assert_eq!(PROTOCOL_VERSION, 1);
        assert_eq!(serde_json::to_value(ProtocolVersion).unwrap(), json!(1));
        assert_eq!(ProtocolVersion.to_string(), "1");
        serde_json::from_value::<ProtocolVersion>(json!(1)).unwrap();
        for bad in [
            json!(0),
            json!(2),
            json!(u64::MAX),
            json!(-1),
            json!("1"),
            json!(1.5),
            json!(null),
        ] {
            let err = serde_json::from_value::<ProtocolVersion>(bad.clone()).unwrap_err();

            assert_eq!(err.classify(), serde_json::error::Category::Data, "{bad}");
        }
    }

    #[test]
    fn a_request_is_one_line_ending_with_a_newline_with_the_version_first() {
        let request = Request::new(
            RequestId(7),
            Client::Cli,
            Command::Status(StatusRequest { target: None }),
        );

        let line = request.to_line().unwrap();

        assert_eq!(line, format!("{}\n", request_line("{\"command\":\"status\"}")));
        assert_eq!(Request::from_line(&line).unwrap(), request);
    }

    #[test]
    fn a_line_may_end_with_a_newline_or_a_carriage_return_and_a_newline_or_nothing() {
        let body = request_line("{\"command\":\"status\"}");

        for line in [body.clone(), format!("{body}\n"), format!("{body}\r\n")] {
            Request::from_line(&line).unwrap();
        }
    }

    #[test]
    fn text_with_newlines_stays_on_one_line() {
        let text = SayText::try_from(String::from("first\nsecond\r\nthird")).unwrap();
        let request = Request::new(
            RequestId(1),
            Client::Cli,
            Command::Say(SayRequest {
                target: Target::Team(team_id(1)),
                text,
            }),
        );

        let line = request.to_line().unwrap();

        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
        assert_eq!(Request::from_line(&line).unwrap(), request);
    }

    #[test]
    fn a_line_of_another_protocol_version_is_refused_with_that_version_whatever_else_it_says() {
        for (line, found) in [
            (
                "{\"protocol\":2,\"id\":1,\"client\":{\"kind\":\"cli\"},\"request\":{\"command\":\"status\"}}",
                2,
            ),
            ("{\"protocol\":0}", 0),
            ("{\"protocol\":18446744073709551615}", u64::MAX),
            ("{\"protocol\":3,\"request\":{\"command\":\"warp\",\"speed\":9}}", 3),
            ("{\"id\":1,\"protocol\":2,\"anything\":[1,2,3]}", 2),
        ] {
            let request = Request::from_line(line).unwrap_err();
            let frame = Frame::from_line(line).unwrap_err();

            assert!(
                matches!(&request, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: f }) if *f == found),
                "{line}: {request:?}"
            );
            assert!(
                matches!(&frame, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: f }) if *f == found),
                "{line}: {frame:?}"
            );
        }
    }

    #[test]
    fn a_line_without_a_usable_version_or_not_json_is_malformed() {
        for line in [
            "",
            "  ",
            "not json",
            "[]",
            "null",
            "{}",
            "{\"id\":1}",
            "{\"protocol\":\"1\"}",
            "{\"protocol\":-1}",
            "{\"protocol\":1.0}",
            "{\"protocol\":null}",
            "{\"protocol\":1}",
            "{\"protocol\":1,\"id\":1,\"client\":{\"kind\":\"cli\"},\"request\":{\"command\":\"status\"}} trailing",
        ] {
            let request = Request::from_line(line).unwrap_err();
            let frame = Frame::from_line(line).unwrap_err();

            assert!(
                matches!(request, ProtocolError::Malformed { .. }),
                "{line:?}: {request:?}"
            );
            assert!(matches!(frame, ProtocolError::Malformed { .. }), "{line:?}: {frame:?}");
        }
    }

    #[test]
    fn a_request_that_is_not_one_of_the_commands_is_malformed() {
        for body in [
            "{\"command\":\"reboot\"}",
            "{}",
            "{\"command\":5}",
            "{\"command\":\"status\",\"extra\":1}",
            "{\"command\":\"stop\"}",
            "{\"command\":\"stop\",\"target\":{}}",
            "{\"command\":\"stop\",\"target\":{\"agent\":\"codex-1\"}}",
            "{\"command\":\"stop\",\"target\":{\"team\":\"task-20261111-153001-k3x9\"}}",
            "{\"command\":\"stop\",\"target\":{\"task\":\"team-20261111-153001-t5m8\"}}",
        ] {
            let err = Request::from_line(&request_line(body)).unwrap_err();

            assert!(matches!(err, ProtocolError::Malformed { .. }), "{body}: {err:?}");
        }
        for line in [
            "{\"protocol\":1,\"client\":{\"kind\":\"cli\"},\"request\":{\"command\":\"status\"}}",
            "{\"protocol\":1,\"id\":1,\"request\":{\"command\":\"status\"}}",
            "{\"protocol\":1,\"id\":1,\"client\":{\"kind\":\"cli\"}}",
            "{\"protocol\":1,\"id\":1,\"client\":{\"kind\":\"robot\"},\"request\":{\"command\":\"status\"}}",
            "{\"protocol\":1,\"id\":-1,\"client\":{\"kind\":\"cli\"},\"request\":{\"command\":\"status\"}}",
            "{\"protocol\":1,\"id\":1,\"client\":{\"kind\":\"cli\"},\"request\":{\"command\":\"status\"},\"extra\":1}",
        ] {
            let err = Request::from_line(line).unwrap_err();

            assert!(matches!(err, ProtocolError::Malformed { .. }), "{line}: {err:?}");
        }
    }

    #[test]
    fn a_malformed_line_keeps_the_parsers_reason_as_its_source() {
        let err = Request::from_line("{\"protocol\":1}").unwrap_err();

        let source = std::error::Error::source(&err).expect("a source");

        assert!(source.to_string().contains("missing field"), "{source}");
    }

    #[test]
    fn a_line_over_the_limit_is_refused_before_it_is_read() {
        let request = " ".repeat(MAX_REQUEST_BYTES + 1);
        let frame = " ".repeat(MAX_FRAME_BYTES + 1);

        let too_long_request = Request::from_line(&request).unwrap_err();
        let too_long_frame = Frame::from_line(&frame).unwrap_err();

        assert!(
            matches!(too_long_request, ProtocolError::TooLong { len, max } if len == MAX_REQUEST_BYTES + 1 && max == MAX_REQUEST_BYTES)
        );
        assert!(
            matches!(too_long_frame, ProtocolError::TooLong { len, max } if len == MAX_FRAME_BYTES + 1 && max == MAX_FRAME_BYTES)
        );
        // A request line is not a frame the daemon would send, and a client may send what a frame could hold.
        assert!(matches!(
            Frame::from_line(&request).unwrap_err(),
            ProtocolError::Malformed { .. }
        ));
    }

    #[test]
    fn a_line_of_exactly_the_limit_is_read() {
        let request = request_line("{\"command\":\"status\"}");
        let padded = format!("{request}{}", " ".repeat(MAX_REQUEST_BYTES - request.len()));
        let frame = Frame::Response(Response::ok(RequestId(3), Reply::Subscribed { teams: vec![] }))
            .to_line()
            .unwrap();
        let padded_frame = format!("{frame}{}", " ".repeat(MAX_FRAME_BYTES - frame.len()));

        assert_eq!(padded.len(), MAX_REQUEST_BYTES);
        assert_eq!(padded_frame.len(), MAX_FRAME_BYTES);
        Request::from_line(&padded).unwrap();
        Frame::from_line(&padded_frame).unwrap();
    }

    #[test]
    fn a_task_list_of_the_largest_size_fits_in_a_request_line() {
        let list = format!("{}\n", "x".repeat(crate::task_list::MAX_TASK_LIST_BYTES - 1));
        let text = crate::state::TaskListText::new(list).unwrap();
        let request = Request::new(
            RequestId(1),
            Client::Cli,
            Command::Run(RunRequest {
                repo: "team-agents".parse().unwrap(),
                checkout: "/home/u/team-agents".parse().unwrap(),
                agents: AgentList::try_from(vec!["codex-1".parse().unwrap()]).unwrap(),
                task_list: text,
                coordinator: None,
                merge: crate::state::MergeMode::Autopilot,
            }),
        );

        let line = request.to_line().unwrap();

        assert!(line.len() <= MAX_REQUEST_BYTES, "{}", line.len());
        assert_eq!(Request::from_line(&line).unwrap(), request);
    }

    // ---- frames -------------------------------------------------------------------------------------------------

    #[test]
    fn a_response_and_an_event_are_told_apart_by_their_type() {
        let response = Frame::Response(Response::ok(RequestId(3), Reply::Subscribed { teams: vec![] }));
        let event = Frame::Event(Event::new(at(0), EventKind::TeamStarted { team: team_id(1) }));

        let response_line = response.to_line().unwrap();
        let event_line = event.to_line().unwrap();

        assert_eq!(
            response_line,
            "{\"type\":\"response\",\"protocol\":1,\"id\":3,\"result\":{\"status\":\"ok\",\"reply\":{\"kind\":\"subscribed\",\"teams\":[]}}}\n"
        );
        assert_eq!(
            event_line,
            "{\"type\":\"event\",\"protocol\":1,\"at\":\"2026-11-11T19:15:00Z\",\"event\":{\"kind\":\"team_started\",\"team\":\"team-20261111-153001-t5m8\"}}\n"
        );
        assert_eq!(Frame::from_line(&response_line).unwrap(), response);
        assert_eq!(Frame::from_line(&event_line).unwrap(), event);
    }

    #[test]
    fn a_frame_of_an_unknown_type_is_malformed() {
        for line in [
            "{\"type\":\"request\",\"protocol\":1}",
            "{\"protocol\":1,\"id\":3,\"result\":{\"status\":\"ok\",\"reply\":{\"kind\":\"subscribed\"}}}",
            "{\"type\":\"response\",\"protocol\":1,\"id\":3}",
        ] {
            let err = Frame::from_line(line).unwrap_err();

            assert!(matches!(err, ProtocolError::Malformed { .. }), "{line}: {err:?}");
        }
    }
}
