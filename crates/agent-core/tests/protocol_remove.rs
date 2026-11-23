//! The `remove` request of the control protocol: it round-trips as a line, and `force` is only written when it is set.

#[cfg(test)]
mod protocol_remove {
    use agent_core::ids::TeamId;
    use agent_core::protocol::{Client, Command, RemoveRequest, Request, RequestId};

    fn team() -> TeamId {
        "team-20261111-153001-t5m8".parse().unwrap()
    }

    fn request(force: bool) -> Request {
        Request::new(
            RequestId(9),
            Client::Cli,
            Command::Remove(RemoveRequest { team: team(), force }),
        )
    }

    #[test]
    fn a_remove_request_reads_back_as_the_same_line() {
        let line = request(true).to_line().unwrap();

        let read = Request::from_line(&line).unwrap();

        assert_eq!(read, request(true));
        assert_eq!(read.to_line().unwrap(), line);
    }

    #[test]
    fn a_remove_request_without_force_writes_no_force_field() {
        let line = request(false).to_line().unwrap();

        assert!(!line.contains("force"), "{line}");
        assert_eq!(Request::from_line(&line).unwrap(), request(false));
    }
}
