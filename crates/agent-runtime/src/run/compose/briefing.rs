//! The briefing of a task started in a new session: the facts of the work the runs before it left behind, as the text
//! of [`HarnessPrompt::Briefing`](agent_core::prompt::HarnessPrompt::Briefing).

use agent_core::prompt::Variables;

/// A commit of the task's branch since its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingCommit<'a> {
    /// The short ID of the commit.
    pub id: &'a str,
    /// The first line of its message.
    pub subject: &'a str,
}

/// A step of the task's plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingStep<'a> {
    /// The step's ID, such as `1.2`.
    pub id: &'a str,
    /// The rest of the step's line.
    pub title: &'a str,
    /// The step's state, as the plan writes it: `to do`, `in hand`, `done` or `given up`.
    pub state: &'a str,
}

/// A message of the task's team that no run of the task had read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingMessage<'a> {
    /// The message's number on the bus.
    pub id: &'a str,
    /// Who sent it.
    pub from: &'a str,
    /// Its text.
    pub text: &'a str,
}

/// The facts of the work that a new session of a task is briefed with.
#[derive(Debug, Clone, Copy)]
pub struct Briefing<'a> {
    /// Why the task is started anew, as one line for the agent.
    pub reason: &'a str,
    /// The task.
    pub task: &'a str,
    /// The task's own prompt.
    pub prompt: &'a str,
    /// The branch the work is forked from.
    pub base: &'a str,
    /// The branch the task works on.
    pub branch: &'a str,
    /// The commits on the branch since the base, oldest first.
    pub commits: &'a [BriefingCommit<'a>],
    /// The files the worktree holds that no commit does.
    pub uncommitted: &'a [&'a str],
    /// The task's plan of steps, when it has one.
    pub plan: Option<&'a [BriefingStep<'a>]>,
    /// The messages of the task's team it had not read.
    pub messages: &'a [BriefingMessage<'a>],
    /// The reference of the snapshot of the worktree, taken before the session starts.
    pub snapshot: &'a str,
}

/// The briefing of `facts`, rendered through `render`.
#[must_use]
pub fn briefing(render: &dyn Fn(&str, &Variables) -> String, facts: &Briefing<'_>) -> String {
    let _ = (render, facts);
    String::new()
}
