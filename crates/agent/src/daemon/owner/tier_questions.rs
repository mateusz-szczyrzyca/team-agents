//! One human question per durable wait for capability absent from the crew.

use agent_core::ids::TeamId;
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
use agent_core::state::Timestamp;
use agent_core::status::TaskStatus;
use agent_runtime::team_prompts::RenderTeamTextError;
use agent_runtime::text_file::{ReplaceError, replace_text_file};

use super::{Asked, Owner, OwnerError};

/// A capability question could not be prepared for the human.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TierQuestionError {
    /// Validated item labels could not be supplied to the catalog.
    #[error("cannot prepare the capability question")]
    Variables(#[from] VariablesError),
    /// The installed catalog cannot render the question.
    #[error("cannot render the capability question")]
    Render(#[from] RenderTeamTextError),
    /// The file cannot be atomically written in the task directory.
    #[error("cannot write the capability question")]
    Write(#[from] ReplaceError),
}

impl Owner {
    /// Records the question and its asked markers together before the next run can start.
    pub(super) async fn ask_tier_waits(&mut self, team_id: &TeamId) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let Some(task) = team.tier_question_task() else {
            return Ok(());
        };
        if self.in_flight.contains(&task) {
            return Ok(());
        }
        let text = self.tier_question_text(team_id, &task)?;
        let directory = self.parts.store.home().task_dir(&task);
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|source| agent_runtime::store::StoreError::CreateDir {
                path: directory,
                source,
            })?;
        let number = super::start::next_number(&self.member_mut(&task)?.1)
            .ok_or_else(|| OwnerError::QuestionNumbers { task: task.clone() })?;
        replace_text_file(&self.parts.store.home().questions_file(&task, number), &text, 0o600)
            .await
            .map_err(TierQuestionError::Write)?;
        let located = self.locate(&task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        let (status, notice) = match self.ask(&task, &located, now, &mut events).await? {
            Asked::Recorded(notice) => {
                self.parts
                    .state
                    .team_mut(team_id)
                    .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?
                    .mark_tier_questions();
                (TaskStatus::Questions, notice)
            }
            Asked::Unusable(notice) => (TaskStatus::Failed, notice),
        };
        // The harness performs this turn itself: use the legal task transitions without starting a CLI.
        if self.member_mut(&task)?.1.status() == TaskStatus::Waiting {
            self.move_to(&task, team_id, TaskStatus::Queued, &mut events)?;
        }
        self.move_to(&task, team_id, TaskStatus::Running, &mut events)?;
        self.move_to(&task, team_id, status, &mut events)?;
        self.commit(&task, &located, now, events, Some(notice)).await
    }

    /// Renders the blocked parts and their minimum tiers from the current snapshot.
    fn tier_question_text(&self, team_id: &TeamId, task: &agent_core::ids::TaskId) -> Result<String, OwnerError> {
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let mut waiting = String::new();
        for (item, tier) in team.unasked_tier_waits() {
            let label = team
                .round_of(item)
                .and_then(agent_core::state::Round::part)
                .map_or_else(|| item.to_string(), |part| format!("{item}{part}"));
            let values = Variables::new()
                .with("item", label)
                .and_then(|values| values.with("tier", tier.as_str()))
                .map_err(TierQuestionError::Variables)?;
            waiting.push_str(
                &self
                    .parts
                    .team_texts
                    .render(TeamPrompt::TierWaiting, &values)
                    .map_err(TierQuestionError::Render)?,
            );
        }
        let values = Variables::new()
            .with("task", task.as_str())
            .and_then(|values| values.with("waiting", waiting.trim_end()))
            .map_err(TierQuestionError::Variables)?;
        self.parts
            .team_texts
            .render(TeamPrompt::NoticeTierQuestion, &values)
            .map_err(TierQuestionError::Render)
            .map_err(OwnerError::from)
    }
}
