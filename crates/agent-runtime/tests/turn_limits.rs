//! Configured turn budgets affect only CLIs with a per-invocation limit.

use agent_core::cli::{RunMode, RunRequest, SessionId};
use agent_core::config::CliArgs;
use agent_core::kinds::CliKind;
use agent_runtime::cli::adapter_for;
use std::collections::BTreeMap;
use std::num::NonZeroU32;

/// A request without extra arguments, for either a new or resumed session.
fn request(mode: RunMode) -> Result<RunRequest, Box<dyn std::error::Error>> {
    Ok(RunRequest {
        program: "/opt/bin/fake-cli".parse()?,
        model: None,
        extra_args: CliArgs::default(),
        mode,
        prompt: String::new(),
        working_dir: "/work/tree".parse()?,
        run_dir: "/work/run".parse()?,
        mcp_servers: BTreeMap::new(),
    })
}

#[test]
fn claude_new_and_resumed_commands_use_the_configured_turn_budget() {
    for turns in [1, 50, 300, u32::MAX] {
        let session_id: SessionId = "12345678-1234-1234-1234-123456789abc".parse().unwrap();
        for mode in [
            RunMode::New {
                session_id: session_id.clone(),
            },
            RunMode::Resume { session_id },
        ] {
            let request = request(mode).unwrap();
            let spec = adapter_for(CliKind::Claude)
                .build_run_with_max_turns(&request, NonZeroU32::new(turns).unwrap())
                .unwrap();
            let actual: Vec<_> = spec
                .args
                .windows(2)
                .filter(|pair| pair[0] == "--max-turns")
                .map(|pair| pair[1].as_str())
                .collect();
            assert_eq!(actual, [turns.to_string()], "turns={turns}");
        }
    }
}

#[test]
fn adapters_without_a_turn_limit_keep_their_commands_unchanged() {
    for cli in [CliKind::Codex, CliKind::Gemini, CliKind::Opencode] {
        let request = request(RunMode::New {
            session_id: "session-1".parse().unwrap(),
        })
        .unwrap();
        let adapter = adapter_for(cli);
        let ordinary = adapter.build_run(&request).unwrap();
        let configured = adapter
            .build_run_with_max_turns(&request, NonZeroU32::new(50).unwrap())
            .unwrap();
        assert_eq!(ordinary, configured, "{cli:?}");
    }
}
