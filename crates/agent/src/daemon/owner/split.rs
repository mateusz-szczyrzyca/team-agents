//! Coordinator proposal submission and whole-plan activation.

use agent_core::decide::split::approval::{ProposalDecision, ProposalRead, decide_proposal};
use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::questions::MAX_QUESTIONS_BYTES;
use agent_core::state::split::proposal::{ReadProposalError, read_proposal};
use agent_core::status::TaskStatus;
use agent_runtime::text_file::{TextFileError, read_text_file};

use super::{Located, Owner, OwnerError};
use crate::daemon::settle::Settled;

/// A submitted proposal could not be read or interpreted.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ProposalError {
    /// The task's proposal file could not be read.
    #[error("cannot read the proposal file {path:?}")]
    Read {
        /// File named by the submission protocol.
        path: std::path::PathBuf,
        /// Bounded file reader's failure.
        source: TextFileError,
    },
    /// The proposal does not follow its JSON form.
    #[error("cannot parse the proposal")]
    Form(#[from] ReadProposalError),
}

impl Owner {
    /// Recognizes an explicit coordinator submission, leaving unrelated run endings alone.
    pub(super) async fn split_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        message: Option<&str>,
    ) -> Result<Option<Settled>, OwnerError> {
        if located.role != Role::Coordinator {
            return Ok(None);
        }
        let team = self
            .parts
            .state
            .team(&located.team)
            .ok_or_else(|| OwnerError::LostTeam {
                team: located.team.clone(),
            })?;
        let submitted = team.items().iter().find(|item| {
            message.is_some_and(|text| text.lines().any(|line| line.trim() == format!("SPLIT: {}", item.id())))
        });
        let Some(item) = submitted else { return Ok(None) };
        let path = self
            .parts
            .store
            .home()
            .task_dir(task)
            .join(format!("split-proposal-{}.json", item.id()));
        let Some(file) = read_text_file(&path, MAX_QUESTIONS_BYTES)
            .await
            .map_err(|source| ProposalError::Read { path, source })?
        else {
            return Ok(Some(Settled::Status(TaskStatus::Waiting)));
        };
        let proposal = read_proposal(&file.text).map_err(ProposalError::Form)?;
        if let ProposalDecision::Approve(plan) = decide_proposal(
            ProposalRead::Parsed(&proposal),
            self.parts.settings.config.split(),
            "lines",
            0,
        ) {
            self.parts
                .state
                .team_mut(&located.team)
                .ok_or_else(|| OwnerError::LostTeam {
                    team: located.team.clone(),
                })?
                .activate_split(plan)?;
            return Ok(Some(Settled::Status(TaskStatus::Queued)));
        }
        Ok(Some(Settled::Status(TaskStatus::Waiting)))
    }
}
