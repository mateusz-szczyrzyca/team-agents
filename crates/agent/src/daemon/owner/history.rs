//! What the owner records in the history: a team starting, a round opening, a run starting and ending, a control used
//! while a run is going. Each is handed to the history after the state holds it; the history never refuses or delays.

use agent_core::cli::ActivityEvent;
use agent_core::history::{AgentIdentity, Record, RunEndRecord, RunStart, RunWhy};
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::protocol::RunRequest;
use agent_core::state::{RunState, Timestamp};
use agent_runtime::process::error_chain;

use super::Owner;
use super::start::{Plan, work_branch};
use crate::daemon::history::{round_opening, team_start};

impl Owner {
    /// Records the start of the team `team` that `request` asked for, and the rounds it opened with.
    pub(super) fn record_team_start(&self, request: &RunRequest, team: &TeamId) {
        if self.history.is_disabled() {
            return;
        }
        let Some(found) = self.parts.state.team(team) else {
            return;
        };
        let Some(base) = self
            .parts
            .settings
            .config
            .repo(found.repo())
            .map(|repo| &repo.base_branch)
        else {
            tracing::warn!(team = %team, "the repository of a new team is not in the configuration: the history has no record of the team");
            return;
        };
        match team_start(request, found, base, &self.parts.settings.config) {
            Ok(record) => self.history.record(Record::Team(Box::new(record))),
            Err(error) => {
                tracing::warn!(team = %team, error = %error_chain(&error), "the plan of a new team cannot be explained: the history has no record of the team");
            }
        }
        for round in found.rounds() {
            self.record_round(team, round.id());
        }
    }

    /// Records the opening of `round` of `team`.
    pub(super) fn record_round(&self, team: &TeamId, round: &RoundId) {
        let Some(found) = self.parts.state.team(team) else {
            return;
        };
        let Some(opening) = found.round(round).and_then(|round| round_opening(found, round)) else {
            tracing::warn!(team = %team, round = %round, "the item of a round is not in the task list: the history has no record of the round");
            return;
        };
        self.history.record(Record::Round(Box::new(opening)));
    }

    /// Records the start of the run `plan` carries out for `task`, which works for `rounds` because of `why`, at `at`.
    pub(super) fn record_run_start(&self, task: &TaskId, plan: &Plan, rounds: &[RoundId], why: RunWhy, at: Timestamp) {
        if self.history.is_disabled() {
            return;
        }
        let Some(found) = self.parts.state.find_task(task) else {
            return;
        };
        let (team, member) = (found.team, found.member);
        let branch = match work_branch(team, member) {
            Ok(branch) => branch,
            Err(error) => {
                tracing::warn!(task = %task, error = %error_chain(&error), "the branch of a run cannot be told: the history has no record of the run");
                return;
            }
        };
        let participant = member.agent();
        self.history.record(Record::Run(Box::new(RunStart {
            run: plan.run.clone(),
            team: team.id().clone(),
            task: task.clone(),
            at,
            agent: AgentIdentity {
                name: participant.name().to_owned(),
                tier: participant.tier(),
                cli: plan.job.cli,
                model: plan.job.model.clone(),
            },
            role: found.role,
            rounds: rounds.to_vec(),
            why,
            prompt: plan.job.prompt.clone(),
            worktree: plan.job.working_dir.clone(),
            branch,
            session: plan.recorded_session.clone(),
        })));
    }

    /// Records how `run` ended at `at`, with the agent's `final_message`, and forgets the numbering of its steps.
    pub(super) fn record_run_end(
        &self,
        run: &RunId,
        at: Timestamp,
        end: &agent_core::state::RunEnd,
        final_message: Option<&str>,
    ) {
        self.history.record(Record::RunEnd(Box::new(RunEndRecord {
            run: run.clone(),
            at,
            end: end.clone(),
            final_message: final_message.map(str::to_owned),
        })));
        self.history.finish_run(run);
    }

    /// Records the end of the runs that a restart found going: each was ended with no status and no exit code.
    pub(super) fn record_recovered_ends(&self, tasks: &[TaskId]) {
        for task in tasks {
            let Some(found) = self.parts.state.find_task(task) else {
                continue;
            };
            let Some(run) = found.member.runs().last() else {
                continue;
            };
            if let RunState::Ended { at, end } = run.state() {
                self.record_run_end(run.id(), *at, end, None);
            }
        }
    }

    /// Records `event`, a control of the human, as a step of the run of `task` that is going, if one is.
    pub(super) fn record_control_step(&self, task: &TaskId, at: Timestamp, event: &ActivityEvent) {
        let running = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.current_run().map(|run| run.id().clone()));
        if let Some(run) = running {
            self.history.step(&run, at, event.clone());
        }
    }
}
