//! The Codex adapter.

use agent_core::cli::{BuildRunError, CliAdapter, OutputParser, RunRequest, RunSpec};
use agent_core::kinds::CliKind;

use super::{StubParser, stub_spec};

/// The adapter of Codex.
#[derive(Debug, Clone, Copy)]
pub struct CodexAdapter;

impl CliAdapter for CodexAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Codex
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        Ok(stub_spec(request))
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(StubParser)
    }
}
