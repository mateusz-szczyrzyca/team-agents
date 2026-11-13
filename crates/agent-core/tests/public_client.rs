//! The client of a request is strict like the rest of the protocol: the unit variant `cli` refuses fields too, so a
//! line from a newer build does not load with part of it silently dropped.

#[cfg(test)]
mod public_client {
    use agent_core::protocol::{Client, ProtocolError, Request};
    use serde::Deserialize;
    use serde_json::{Value, json};

    /// Asserts that `Client::deserialize(value)`, the call a caller can write, fails with a data error.
    fn assert_client_refused(value: &Value) {
        let err = Client::deserialize(value.clone()).expect_err(&format!("{value} should be refused"));
        assert_eq!(err.classify(), serde_json::error::Category::Data, "{value}: {err}");
    }

    /// A request line whose client is `client`.
    fn request_of(client: &Value) -> String {
        json!({"protocol": 2, "id": 1, "client": client, "request": {"command": "status"}}).to_string()
    }

    #[test]
    fn a_request_from_the_cli_with_an_unknown_field_is_refused() {
        let line = request_of(&json!({"kind": "cli", "unexpected": true}));

        let err = Request::from_line(&line).unwrap_err();

        assert!(matches!(err, ProtocolError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn a_request_from_the_cli_with_a_dashboard_login_is_refused() {
        let line = request_of(&json!({"kind": "cli", "login": "you@example.com"}));

        let err = Request::from_line(&line).unwrap_err();

        assert!(matches!(err, ProtocolError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn a_plain_cli_client_is_still_read_and_written_the_way_it_was() {
        let line = request_of(&json!({"kind": "cli"}));

        let request = Request::from_line(&line).unwrap();

        assert_eq!(request.client, Client::Cli);
        let written: Value = serde_json::from_str(&request.to_line().unwrap()).unwrap();
        assert_eq!(written, serde_json::from_str::<Value>(&line).unwrap());
    }

    #[test]
    fn a_direct_deserialize_of_a_client_refuses_an_unknown_field() {
        assert_client_refused(&json!({"kind": "cli", "extra": 1}));
        assert_client_refused(&json!({"kind": "cli", "login": "you@example.com"}));
        assert_client_refused(&json!({"kind": "dashboard", "login": "you@example.com", "extra": 1}));
        assert_eq!(Client::deserialize(json!({"kind": "cli"})).unwrap(), Client::Cli);
    }
}
