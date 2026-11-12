//! The Antigravity (agy) adapter.

use agent_core::cli::{BuildRunError, CliAdapter, OutputParser, RunRequest, RunSpec};
use agent_core::kinds::CliKind;

use super::{StubParser, stub_spec};

/// The adapter of Antigravity (agy).
#[derive(Debug, Clone, Copy)]
pub struct AgyAdapter;

impl CliAdapter for AgyAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Gemini
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        Ok(stub_spec(request))
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(StubParser)
    }
}
