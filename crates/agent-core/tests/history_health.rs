//! How the health of the history reads: what `agent status` says when the history is not whole, and the line of the
//! protocol that carries it.

#[cfg(test)]
mod history_health {
    use agent_core::history::{HistoryHealth, HistoryState};
    use agent_core::protocol::{Frame, Reply, RequestId, Response};

    fn health(state: HistoryState, lost: u64) -> HistoryHealth {
        HistoryHealth { state, lost }
    }

    #[test]
    fn the_health_is_a_line_that_says_what_the_history_is_doing_and_how_many_records_it_lost() {
        let unavailable = HistoryState::Unavailable {
            reason: "the file is a directory".to_owned(),
        };
        for (health, text) in [
            (health(HistoryState::Writing, 0), "history: written"),
            (health(HistoryState::Writing, 1), "history: written; 1 record lost"),
            (health(HistoryState::Writing, 7), "history: written; 7 records lost"),
            (
                health(HistoryState::Disabled, 0),
                "history: off ([history] enabled = false)",
            ),
            (
                health(unavailable.clone(), 0),
                "history: not written: the file is a directory",
            ),
            (
                health(unavailable, 12),
                "history: not written: the file is a directory; 12 records lost",
            ),
        ] {
            assert_eq!(health.to_string(), text);
        }
    }

    #[test]
    fn an_overview_reply_is_a_line_of_json_that_reads_back() {
        let reply = Reply::Overview {
            teams: Vec::new(),
            history: health(
                HistoryState::Unavailable {
                    reason: "no disk".to_owned(),
                },
                3,
            ),
        };
        let line = Frame::Response(Response::ok(RequestId(4), reply.clone()))
            .to_line()
            .unwrap();

        assert_eq!(
            line.trim_end(),
            r#"{"type":"response","protocol":2,"id":4,"result":{"status":"ok","reply":{"kind":"overview","teams":[],"history":{"state":{"kind":"unavailable","reason":"no disk"},"lost":3}}}}"#
        );
        assert_eq!(
            Frame::from_line(&line).unwrap(),
            Frame::Response(Response::ok(RequestId(4), reply))
        );
    }
}
