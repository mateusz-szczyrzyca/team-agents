//! Remove configured credentials before records reach the store, without changing CLI input.

use agent_core::cli::ActivityEvent;
use agent_core::config::{Secret, Secrets};
use agent_core::decide::checks::Change;
use agent_core::history::{HowChosen, Record, RoundSubject, RunStart, RunWhy, TeamStart};
use agent_core::protocol::Client;

/// The explicit replacement for a credential in free text.
const MARKER: &str = "[REDACTED]";

/// Known values retain `Secret`'s redacted debug representation even in writer diagnostics.
#[derive(Debug, Default)]
pub(super) struct Redactor {
    /// All credentials configured for GitHub, model CLIs and notifications.
    secrets: Secrets,
}

impl Redactor {
    /// Owns the configured credentials for the writer's lifetime.
    pub(super) const fn new(secrets: Secrets) -> Self {
        Self { secrets }
    }

    /// Values retain their protected type until matching is necessary.
    fn values(&self) -> impl Iterator<Item = &Secret> {
        self.secrets
            .github_token()
            .into_iter()
            .chain(self.secrets.agent_env().values())
            .chain(self.secrets.notification_env().values())
    }

    /// Whether `text` contains a configured value, without printing the value in diagnostics.
    fn contains(&self, text: &str) -> bool {
        self.values().any(|value| text.contains(value.expose()))
    }

    /// Replaces the union of overlapping matches, so a shorter match cannot hide a longer one.
    pub(super) fn text(&self, text: &str) -> String {
        if !self.contains(text) {
            return text.to_owned();
        }
        let mut result = String::with_capacity(text.len());
        let mut covered = 0;
        for (at, character) in text.char_indices() {
            let end = self
                .values()
                .filter_map(|value| {
                    text.get(at..)
                        .is_some_and(|rest| rest.starts_with(value.expose()))
                        .then_some(at + value.expose().len())
                })
                .max()
                .unwrap_or(at);
            if end > at {
                if at >= covered {
                    result.push_str(MARKER);
                }
                covered = covered.max(end);
            }
            if at >= covered {
                result.push(character);
            }
        }
        result
    }

    /// Redacts free text; returns false if a credential occurs in an identity that cannot be changed safely.
    pub(super) fn record(&self, record: &mut Record) -> bool {
        if self.values().next().is_none() {
            return true;
        }
        // A replacement must itself be safe. An unusable marker fails closed rather than leaking.
        if self.contains(MARKER) {
            return false;
        }
        match record {
            Record::Team(team) => {
                team.task_list = self.text(&team.task_list);
                self.team_safe(team)
            }
            Record::Round(round) => {
                match &mut round.subject {
                    RoundSubject::Brief(text) => *text = self.text(text),
                    RoundSubject::Item(item) => {
                        item.title = self.text(&item.title);
                        item.description = self.text(&item.description);
                        item.expected_result = item.expected_result.as_ref().map(|text| self.text(text));
                        item.checks = item.checks.as_ref().map(|text| self.text(text));
                    }
                }
                !self.contains(round.team.as_str()) && !self.contains(round.round.as_str())
            }
            Record::Run(run) => {
                run.prompt = self.text(&run.prompt);
                self.run_safe(run)
            }
            Record::Launch(launch) => {
                for arg in &mut launch.args {
                    *arg = self.text(arg);
                }
                !self.contains(launch.run.as_str())
                    && !self.contains(&launch.program)
                    && !self.contains(&launch.working_dir.as_path().to_string_lossy())
                    && !launch.head.as_ref().is_some_and(|head| self.contains(head.as_str()))
                    && !launch.env_names.iter().any(|name| self.contains(name.as_str()))
            }
            Record::Step(step) => self.activity(&mut step.event) && !self.contains(step.run.as_str()),
            Record::RunEnd(end) => {
                end.final_message = end.final_message.as_ref().map(|text| self.text(text));
                !self.contains(end.run.as_str())
            }
            Record::Changes(changes) => {
                for commit in &mut changes.commits {
                    commit.subject = self.text(&commit.subject);
                }
                !self.contains(changes.run.as_str())
                    && !changes
                        .head_before
                        .iter()
                        .chain(&changes.head_after)
                        .any(|id| self.contains(id.as_str()))
                    && !changes.commits.iter().any(|commit| self.contains(commit.id.as_str()))
                    && !changes.files.iter().chain(&changes.uncommitted).any(|file| {
                        self.contains(file.path.as_str())
                            || matches!(&file.change,
                            Change::Renamed { from } if self.contains(from.as_str()))
                    })
            }
            Record::Session(session) => {
                !self.contains(session.run.as_str()) && !self.contains(session.session.as_str())
            }
            Record::Event(_) => true,
        }
    }

    /// Keeps participant and draw identities intact, or omits their whole parent record.
    fn team_safe(&self, team: &TeamStart) -> bool {
        !self.contains(team.team.as_str())
            && !self.contains(&team.repo)
            && !self.contains(team.base_branch.as_str())
            && !team.plan.iter().any(|entry| {
                self.contains(entry.agent.as_str())
                    || matches!(&entry.how,
                    HowChosen::Drawn { among } if among.iter().any(|name| self.contains(name.as_str())))
            })
    }

    /// Referential identities are never replaced with misleading values.
    fn run_safe(&self, run: &RunStart) -> bool {
        ![
            run.run.as_str(),
            run.team.as_str(),
            run.task.as_str(),
            &run.agent.name,
            run.branch.as_str(),
        ]
        .iter()
        .any(|text| self.contains(text))
            && !self.contains(&run.worktree.as_path().to_string_lossy())
            && !run
                .agent
                .model
                .as_ref()
                .is_some_and(|model| self.contains(model.as_str()))
            && !run
                .session
                .as_ref()
                .is_some_and(|session| self.contains(session.as_str()))
            && !run.rounds.iter().any(|id| self.contains(id.as_str()))
            && !matches!(&run.why, RunWhy::Request { asked_by: Some(id) } if self.contains(id.as_str()))
    }

    /// Activity payloads are text; the authenticated dashboard identity is not.
    fn activity(&self, event: &mut ActivityEvent) -> bool {
        match event {
            ActivityEvent::Text(text)
            | ActivityEvent::Command(text)
            | ActivityEvent::Error(text)
            | ActivityEvent::Info(text)
            | ActivityEvent::FileEdit { path: text } => *text = self.text(text),
            ActivityEvent::BusCall { tool, arguments } => {
                *tool = self.text(tool);
                *arguments = self.text(arguments);
            }
            ActivityEvent::ToolCall { name, summary } => {
                *name = self.text(name);
                *summary = self.text(summary);
            }
            ActivityEvent::Control {
                by: Client::Dashboard { login },
                ..
            } => return !self.contains(login.as_str()),
            ActivityEvent::Control { by: Client::Cli, .. } => {}
        }
        true
    }
}
