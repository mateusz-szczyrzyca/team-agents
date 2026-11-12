//! The opencode adapter.

use agent_core::cli::{BuildRunError, CliAdapter, OutputParser, RunRequest, RunSpec};
use agent_core::kinds::CliKind;

use super::{StubParser, stub_spec};

/// The adapter of opencode.
#[derive(Debug, Clone, Copy)]
pub struct OpencodeAdapter;

impl CliAdapter for OpencodeAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Opencode
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        Ok(stub_spec(request))
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(StubParser)
    }
}
