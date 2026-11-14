//! Every CLI exposes sessions as soon as it sees them, without consuming its parser.

use agent_core::cli::CliAdapter;
use agent_runtime::cli::{agy::AgyAdapter, claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpencodeAdapter};

#[test]
fn all_adapters_expose_live_session_changes_and_keep_them_after_unrelated_lines() {
    let cases: [(&dyn CliAdapter, &str); 4] = [
        (
            &ClaudeAdapter,
            r#"{"type":"system","subtype":"init","session_id":"SESSION","model":"fake"}"#,
        ),
        (&CodexAdapter, r#"{"type":"thread.started","thread_id":"SESSION"}"#),
        (&AgyAdapter, r#"{"event":"init","conversation_id":"SESSION"}"#),
        (&OpencodeAdapter, r#"{"type":"step_start","sessionID":"SESSION"}"#),
    ];
    for (adapter, line) in cases {
        let mut parser = adapter.parser();
        assert_eq!(parser.session(), None);
        for session in ["session-first", "session-second"] {
            parser.push_line(&line.replace("SESSION", session)).unwrap();
            assert_eq!(
                parser.session().map(agent_core::cli::SessionId::as_str),
                Some(session),
                "{}",
                adapter.kind()
            );
            parser.push_line("").unwrap();
            assert_eq!(parser.session().map(agent_core::cli::SessionId::as_str), Some(session));
        }
    }
}
