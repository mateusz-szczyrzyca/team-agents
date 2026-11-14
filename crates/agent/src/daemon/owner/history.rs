//! What the owner records in the history: a team starting, a round opening, a run starting and ending, a control used
//! while a run is going. Each is handed to the history after the state holds it; the history never refuses or delays.

use agent_core::cli::ActivityEvent;
use agent_core::history::{AgentIdentity, EventRecord, HistoryEvent, Record, RunEndRecord, RunStart, RunWhy};
use agent_core::ids::{ItemId, RoundId, RunId, TaskId, TeamId};
use agent_core::protocol::RunRequest;
use agent_core::state::{ItemPhase, RunState, Timestamp};
use agent_runtime::process::error_chain;

use super::Owner;
use super::start::{Plan, work_branch};
use crate::daemon::history::{round_opening, team_start};

impl Owner {
    /// Records `event` of `team` for `rounds` at the time the clock gives; a time the clock cannot give is a record lost.
    pub(super) fn record_event(&self, team: &TeamId, rounds: Vec<RoundId>, event: HistoryEvent) {
        if self.history.is_disabled() {
            return;
        }
        match Timestamp::now(&self.parts.clock) {
            Ok(at) => self.record_event_at(at, team, rounds, event),
            Err(error) => {
                tracing::warn!(team = %team, error = %error_chain(&error), "the clock gives a time that cannot be recorded: the history has no record of an event");
                self.history.record_lost();
            }
        }
    }

    /// Records `event` of `team` for `rounds` as having happened at `at`.
    pub(super) fn record_event_at(&self, at: Timestamp, team: &TeamId, rounds: Vec<RoundId>, event: HistoryEvent) {
        self.history.record(Record::Event(Box::new(EventRecord {
            at,
            team: team.clone(),
            rounds,
            event,
        })));
    }

    /// The rounds of the `items` of `team`, each once, in the order of the items; an item with no round is told and left out.
    pub(super) fn rounds_of_items(&self, team: &TeamId, items: &[ItemId]) -> Vec<RoundId> {
        let mut rounds: Vec<RoundId> = Vec::new();
        for item in items {
            match self.round_of_item(team, *item) {
                Ok(round) if !rounds.contains(&round) => rounds.push(round),
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(team = %team, item = %item, error = %error_chain(&error), "an item has no round: an event of the history is not linked to it");
                }
            }
        }
        rounds
    }

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
            self.history.record_lost();
            return;
        };
        match team_start(request, found, base, &self.parts.settings.config) {
            Ok(record) => self.history.record(Record::Team(Box::new(record))),
            Err(error) => {
                tracing::warn!(team = %team, error = %error_chain(&error), "the plan of a new team cannot be explained: the history has no record of the team");
                self.history.record_lost();
            }
        }
        for round in found.rounds() {
            self.record_round(team, round.id());
        }
    }

    /// Records the opening of `round` of `team`.
    pub(super) fn record_round(&self, team: &TeamId, round: &RoundId) {
        if self.history.is_disabled() {
            return;
        }
        let Some(found) = self.parts.state.team(team) else {
            return;
        };
        let Some(opening) = found.round(round).and_then(|round| round_opening(found, round)) else {
            tracing::warn!(team = %team, round = %round, "the item of a round is not in the task list: the history has no record of the round");
            self.history.record_lost();
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
                self.history.record_lost();
                return;
            }
        };
        let participant = member.agent();
        let fix_pass = match &why {
            RunWhy::FixPass { number } => Some(*number),
            RunWhy::FirstRun
            | RunWhy::Item { .. }
            | RunWhy::WakeUp
            | RunWhy::Request { .. }
            | RunWhy::Review { .. }
            | RunWhy::AskHuman
            | RunWhy::VerdictMissing
            | RunWhy::Checks
            | RunWhy::Answers
            | RunWhy::ResumedByHuman
            | RunWhy::Continuation => None,
        };
        // The findings a fix pass sends the team are what the autopilot added to the lead's prompt.
        if let (Some(number), Some(findings)) = (fix_pass, &plan.addition) {
            let pr = team.items().iter().find_map(|progress| match progress.phase() {
                ItemPhase::Fixing { assignee, pr, .. } if assignee.as_str() == participant.name() => Some(pr.number),
                ItemPhase::Fixing { .. }
                | ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::InReview { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => None,
            });
            if let Some(pr) = pr {
                self.record_event_at(
                    at,
                    team.id(),
                    rounds.to_vec(),
                    HistoryEvent::FixPassStarted {
                        pr,
                        pass: number,
                        of: self.parts.settings.config.review().fix_passes,
                        findings: findings.clone(),
                    },
                );
            }
        }
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
