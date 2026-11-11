//! The `cli` client of a request is strict like every other part of the line: serde's internally tagged unit variants
//! skip the unknown-field check, so the protocol reads the client through a strict form.

#[cfg(test)]
mod cli_client {
    use agent_core::protocol::{Client, Request};
    use serde_json::json;

    /// A request line from `client`.
    fn line(client: &serde_json::Value) -> String {
        json!({"protocol": 1, "id": 1, "client": client, "request": {"command": "status"}}).to_string()
    }

    #[test]
    fn the_cli_client_reads() {
        let request = Request::from_line(&line(&json!({"kind": "cli"}))).unwrap();

        assert_eq!(request.client, Client::Cli);
    }

    #[test]
    fn the_cli_client_refuses_a_field_it_does_not_have() {
        Request::from_line(&line(&json!({"kind": "cli", "unexpected": true}))).unwrap_err();
    }

    #[test]
    fn the_cli_client_refuses_the_login_of_a_dashboard() {
        Request::from_line(&line(&json!({"kind": "cli", "login": "human@example.com"}))).unwrap_err();
    }
}
