//! The round of one item: member work → coordinator integration → gate green at the head → pull request open → two
//! reviews → merge (or, with `--no-merge`, handed to the human) → item done. "Changes requested" sends the item back
//! to the team for a fix pass, at most the configured number of times; after that the human is asked.
//!
//! A round is a [`RoundState`] that changes only through [`RoundState::transition`]: an event of the world
//! ([`RoundEvent`]) in, the next state and what the daemon is to do about it ([`Step`]) out, or a [`RoundError`] for an
//! event that makes no sense now. The daemon carries the action out and reports the outcome as the next event; nothing
//! here touches git, GitHub or the state file.
//!
//! A solo task is its own integrator: the same states, the same agent in both roles.
//!
//! Fix passes are counted as the state records them (`ItemProgress::fix_passes`): every pass the team starts after a
//! review asked for changes counts, the one the human grants by answering included. So after the human's answer the
//! next "changes requested" asks the human again.

use std::num::NonZeroU32;

use super::REVIEWS_REQUIRED;
use super::checks::{Issue, Judge, Subject};
use crate::config::Review as ReviewConfig;
use crate::ids::InvestigatorName;
use crate::state::{CommitId, MergeMode, PhaseKind, PrNumber};
use crate::status::ReviewVerdict;

mod runs;

pub use self::runs::{RunStart, TaskRounds, rounds_of_run};

/// What a round needs to know besides its state: the rules of the team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundPolicy {
    /// What happens to an approved pull request.
    pub merge: MergeMode,
    /// How many fix passes "changes requested" may start before the human is asked; `0` asks at once.
    pub fix_passes: u32,
}

impl RoundPolicy {
    /// The policy of a team that merges as `merge` says, with the fix passes of the configuration's `[review]`.
    #[must_use]
    pub const fn new(merge: MergeMode, review: &ReviewConfig) -> Self {
        Self {
            merge,
            fix_passes: review.fix_passes,
        }
    }
}

/// Whether the work of the team on an item is the first attempt or a fix pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The first attempt: no pull request exists yet.
    First,
    /// A fix pass on the open pull request.
    Fixing {
        /// The pull request the fixes go into.
        pr: PrNumber,
        /// Which fix pass this is, counted from 1.
        pass: NonZeroU32,
    },
}

/// Where the round of one item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundState {
    /// The assigned member works on the item.
    MemberWork {
        /// The first attempt or a fix pass.
        stage: Stage,
    },
    /// The coordinator integrates the member's branch and gets the gate green at its head.
    Integration {
        /// The first attempt or a fix pass.
        stage: Stage,
    },
    /// The gate is green at `head`: the branch is to be pushed and the pull request opened or updated.
    Publishing {
        /// The first attempt or a fix pass.
        stage: Stage,
        /// The commit the gate passed on, which the pull request must carry.
        head: CommitId,
    },
    /// The pull request is open and waits for its reviews.
    InReview {
        /// The pull request.
        pr: PrNumber,
        /// The fix passes started so far.
        passes: u32,
        /// The investigators that approved, in the order their reviews came.
        approvals: Vec<InvestigatorName>,
    },
    /// Both reviews approve; the pull request waits to be merged by the harness, or with `--no-merge` by the human.
    Approved {
        /// The pull request.
        pr: PrNumber,
    },
    /// The reviewers asked for changes and the fix passes are used up: the human is asked what to do.
    AskingHuman {
        /// The pull request.
        pr: PrNumber,
        /// The fix passes started so far.
        passes: u32,
    },
    /// The pull request is merged and the item is done.
    Done {
        /// The merged pull request.
        pr: PrNumber,
    },
}

/// Which [`RoundState`] a round is in, without its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum RoundPhase {
    /// See [`RoundState::MemberWork`].
    MemberWork,
    /// See [`RoundState::Integration`].
    Integration,
    /// See [`RoundState::Publishing`].
    Publishing,
    /// See [`RoundState::InReview`].
    InReview,
    /// See [`RoundState::Approved`].
    Approved,
    /// See [`RoundState::AskingHuman`].
    AskingHuman,
    /// See [`RoundState::Done`].
    Done,
}

/// Something that happened to the round, as the daemon observes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundEvent {
    /// The member's run reported `DONE`: its branch is ready to be integrated.
    MemberDone,
    /// The coordinator's gate passed at `head`.
    GateGreen {
        /// The commit the gate passed on.
        head: CommitId,
    },
    /// The coordinator's gate failed.
    GateFailed,
    /// The pull request was opened or updated.
    PullRequestPublished {
        /// The pull request.
        pr: PrNumber,
        /// The commit it carries.
        head: CommitId,
    },
    /// A reviewer reported a verdict.
    ReviewSubmitted {
        /// The investigator that reviewed.
        reviewer: InvestigatorName,
        /// What it decided.
        verdict: ReviewVerdict,
    },
    /// The pull request was merged, by the harness or by the human.
    Merged {
        /// The merged pull request.
        pr: PrNumber,
    },
    /// The human answered the question about the used-up fix passes.
    HumanAnswered,
}

/// Which [`RoundEvent`] happened, without its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum RoundEventKind {
    /// See [`RoundEvent::MemberDone`].
    MemberDone,
    /// See [`RoundEvent::GateGreen`].
    GateGreen,
    /// See [`RoundEvent::GateFailed`].
    GateFailed,
    /// See [`RoundEvent::PullRequestPublished`].
    PullRequestPublished,
    /// See [`RoundEvent::ReviewSubmitted`].
    ReviewSubmitted,
    /// See [`RoundEvent::Merged`].
    Merged,
    /// See [`RoundEvent::HumanAnswered`].
    HumanAnswered,
}

/// What the daemon does after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundAction {
    /// Wake the coordinator: merge the member's branch into the round's branch and get the gate green.
    IntegrateBranch,
    /// Tell the coordinator the gate failed: fix what it found and run the gate again.
    FixGate,
    /// Push the branch, open the pull request or update it, and mark it ready.
    PublishPullRequest,
    /// Ask investigators from outside the team for the reviews.
    RequestReviews,
    /// Nothing yet: more reviews are due.
    AwaitReviews,
    /// Give the team the reviewers' findings for this fix pass.
    StartFixPass {
        /// Which fix pass this is, counted from 1.
        pass: NonZeroU32,
    },
    /// Ask the human what to do: the fix passes are used up.
    AskHuman,
    /// Merge the pull request through the API.
    Merge,
    /// Tell the human the pull request is approved and waits for them to merge it (`--no-merge`).
    HandToHuman,
    /// Mark the item done and start what it was holding back.
    CompleteItem,
}

/// The result of one transition: the round's next state and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The state the round is in now.
    pub state: RoundState,
    /// What the daemon does next.
    pub action: RoundAction,
}

/// Why an event does not fit the round.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoundError {
    /// The event makes no sense in this phase.
    #[error("{event} does not fit a round in the {phase} phase")]
    NotExpected {
        /// The phase the round is in.
        phase: RoundPhase,
        /// The event that came.
        event: RoundEventKind,
    },
    /// A reviewer that already approved reported again.
    #[error("{reviewer} already approved this pull request")]
    DuplicateReview {
        /// The reviewer that reported twice.
        reviewer: InvestigatorName,
    },
    /// The pull request carries another commit than the one the gate passed on.
    #[error("the gate passed on {gated}, but the pull request carries {published}")]
    HeadChanged {
        /// The commit the gate passed on.
        gated: CommitId,
        /// The commit the pull request carries.
        published: CommitId,
    },
    /// The event is about another pull request than the round's.
    #[error("the round is about pull request {open}, but the event is about {other}")]
    OtherPullRequest {
        /// The round's pull request.
        open: PrNumber,
        /// The pull request the event names.
        other: PrNumber,
    },
}

impl RoundState {
    /// The state of a round whose item has just been assigned.
    #[must_use]
    pub const fn start() -> Self {
        Self::MemberWork { stage: Stage::First }
    }

    /// Which phase the round is in.
    #[must_use]
    pub const fn phase(&self) -> RoundPhase {
        match self {
            Self::MemberWork { .. } => RoundPhase::MemberWork,
            Self::Integration { .. } => RoundPhase::Integration,
            Self::Publishing { .. } => RoundPhase::Publishing,
            Self::InReview { .. } => RoundPhase::InReview,
            Self::Approved { .. } => RoundPhase::Approved,
            Self::AskingHuman { .. } => RoundPhase::AskingHuman,
            Self::Done { .. } => RoundPhase::Done,
        }
    }

    /// The phase the item is in while the round is in this state: what the state records. Every transition keeps it or
    /// moves along [`PhaseKind::successors`].
    #[must_use]
    pub const fn item_phase(&self) -> PhaseKind {
        match self {
            Self::MemberWork { stage } | Self::Integration { stage } | Self::Publishing { stage, .. } => match stage {
                Stage::First => PhaseKind::InProgress,
                Stage::Fixing { .. } => PhaseKind::Fixing,
            },
            Self::InReview { .. } | Self::AskingHuman { .. } => PhaseKind::InReview,
            Self::Approved { .. } => PhaseKind::Approved,
            Self::Done { .. } => PhaseKind::Done,
        }
    }

    /// Applies `event` to the round.
    ///
    /// # Errors
    /// [`RoundError`] when the event does not fit the state: [`RoundError::NotExpected`] in a phase that does not
    /// take it, [`RoundError::DuplicateReview`], [`RoundError::HeadChanged`] and [`RoundError::OtherPullRequest`]
    /// for data that contradicts what the round knows.
    pub fn transition(&self, event: &RoundEvent, policy: RoundPolicy) -> Result<Step, RoundError> {
        match self {
            Self::MemberWork { stage } => match event {
                RoundEvent::MemberDone => Ok(Step::to(
                    Self::Integration { stage: *stage },
                    RoundAction::IntegrateBranch,
                )),
                RoundEvent::GateGreen { .. }
                | RoundEvent::GateFailed
                | RoundEvent::PullRequestPublished { .. }
                | RoundEvent::ReviewSubmitted { .. }
                | RoundEvent::Merged { .. }
                | RoundEvent::HumanAnswered => Err(self.refuse(event)),
            },
            Self::Integration { stage } => match event {
                RoundEvent::GateGreen { head } => Ok(Step::to(
                    Self::Publishing {
                        stage: *stage,
                        head: head.clone(),
                    },
                    RoundAction::PublishPullRequest,
                )),
                RoundEvent::GateFailed => Ok(Step::to(self.clone(), RoundAction::FixGate)),
                RoundEvent::MemberDone
                | RoundEvent::PullRequestPublished { .. }
                | RoundEvent::ReviewSubmitted { .. }
                | RoundEvent::Merged { .. }
                | RoundEvent::HumanAnswered => Err(self.refuse(event)),
            },
            Self::Publishing { stage, head } => match event {
                RoundEvent::PullRequestPublished { pr, head: published } => publish(*stage, head, *pr, published),
                RoundEvent::MemberDone
                | RoundEvent::GateGreen { .. }
                | RoundEvent::GateFailed
                | RoundEvent::ReviewSubmitted { .. }
                | RoundEvent::Merged { .. }
                | RoundEvent::HumanAnswered => Err(self.refuse(event)),
            },
            Self::InReview { pr, passes, approvals } => match event {
                RoundEvent::ReviewSubmitted { reviewer, verdict } => {
                    review(*pr, *passes, approvals, reviewer, *verdict, policy)
                }
                RoundEvent::MemberDone
                | RoundEvent::GateGreen { .. }
                | RoundEvent::GateFailed
                | RoundEvent::PullRequestPublished { .. }
                | RoundEvent::Merged { .. }
                | RoundEvent::HumanAnswered => Err(self.refuse(event)),
            },
            Self::Approved { pr } => match event {
                RoundEvent::Merged { pr: merged } if merged == pr => {
                    Ok(Step::to(Self::Done { pr: *pr }, RoundAction::CompleteItem))
                }
                RoundEvent::Merged { pr: merged } => Err(RoundError::OtherPullRequest {
                    open: *pr,
                    other: *merged,
                }),
                RoundEvent::MemberDone
                | RoundEvent::GateGreen { .. }
                | RoundEvent::GateFailed
                | RoundEvent::PullRequestPublished { .. }
                | RoundEvent::ReviewSubmitted { .. }
                | RoundEvent::HumanAnswered => Err(self.refuse(event)),
            },
            Self::AskingHuman { pr, passes } => match event {
                RoundEvent::HumanAnswered => Ok(start_fix_pass(*pr, *passes)),
                RoundEvent::MemberDone
                | RoundEvent::GateGreen { .. }
                | RoundEvent::GateFailed
                | RoundEvent::PullRequestPublished { .. }
                | RoundEvent::ReviewSubmitted { .. }
                | RoundEvent::Merged { .. } => Err(self.refuse(event)),
            },
            Self::Done { .. } => Err(self.refuse(event)),
        }
    }

    /// The error for an event that does not fit this state.
    fn refuse(&self, event: &RoundEvent) -> RoundError {
        RoundError::NotExpected {
            phase: self.phase(),
            event: event.kind(),
        }
    }
}

impl RoundEvent {
    /// Which kind of event this is.
    #[must_use]
    pub const fn kind(&self) -> RoundEventKind {
        match self {
            Self::MemberDone => RoundEventKind::MemberDone,
            Self::GateGreen { .. } => RoundEventKind::GateGreen,
            Self::GateFailed => RoundEventKind::GateFailed,
            Self::PullRequestPublished { .. } => RoundEventKind::PullRequestPublished,
            Self::ReviewSubmitted { .. } => RoundEventKind::ReviewSubmitted,
            Self::Merged { .. } => RoundEventKind::Merged,
            Self::HumanAnswered => RoundEventKind::HumanAnswered,
        }
    }
}

impl Step {
    /// A step to `state` that asks for `action`.
    const fn to(state: RoundState, action: RoundAction) -> Self {
        Self { state, action }
    }
}

/// The pull request is published for `published` while the gate passed on `gated`: the reviews start, for the first
/// attempt or for the fix pass `stage` says.
fn publish(stage: Stage, gated: &CommitId, pr: PrNumber, published: &CommitId) -> Result<Step, RoundError> {
    if gated != published {
        return Err(RoundError::HeadChanged {
            gated: gated.clone(),
            published: published.clone(),
        });
    }
    let passes = match stage {
        Stage::First => 0,
        Stage::Fixing { pr: open, .. } if open != pr => return Err(RoundError::OtherPullRequest { open, other: pr }),
        Stage::Fixing { pass, .. } => pass.get(),
    };
    Ok(Step::to(
        RoundState::InReview {
            pr,
            passes,
            approvals: Vec::new(),
        },
        RoundAction::RequestReviews,
    ))
}

/// A reviewer's verdict in a round that has had `passes` fix passes and the approvals of `approvals` so far.
fn review(
    pr: PrNumber,
    passes: u32,
    approvals: &[InvestigatorName],
    reviewer: &InvestigatorName,
    verdict: ReviewVerdict,
    policy: RoundPolicy,
) -> Result<Step, RoundError> {
    match verdict {
        ReviewVerdict::RequestChanges if passes >= policy.fix_passes => {
            Ok(Step::to(RoundState::AskingHuman { pr, passes }, RoundAction::AskHuman))
        }
        ReviewVerdict::RequestChanges => Ok(start_fix_pass(pr, passes)),
        ReviewVerdict::Approve if approvals.contains(reviewer) => Err(RoundError::DuplicateReview {
            reviewer: reviewer.clone(),
        }),
        ReviewVerdict::Approve => {
            let mut approvals = approvals.to_vec();
            approvals.push(reviewer.clone());
            if approvals.len() < REVIEWS_REQUIRED {
                return Ok(Step::to(
                    RoundState::InReview { pr, passes, approvals },
                    RoundAction::AwaitReviews,
                ));
            }
            let action = match policy.merge {
                MergeMode::Autopilot => RoundAction::Merge,
                MergeMode::NoMerge => RoundAction::HandToHuman,
            };
            Ok(Step::to(RoundState::Approved { pr }, action))
        }
    }
}

/// The team starts the fix pass after the `passes` it has had: its work, on the pull request `pr`.
fn start_fix_pass(pr: PrNumber, passes: u32) -> Step {
    let pass = NonZeroU32::MIN.saturating_add(passes);
    Step::to(
        RoundState::MemberWork {
            stage: Stage::Fixing { pr, pass },
        },
        RoundAction::StartFixPass { pass },
    )
}

/// What the checks before a review need to know besides the issues: how often they may send the round back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecksPolicy {
    /// How many times the checks send a round back to the team before the human is asked; `0` asks at once.
    pub check_passes: u32,
}

impl ChecksPolicy {
    /// The policy of the configuration's `[review]` table.
    #[must_use]
    pub const fn new(review: &ReviewConfig) -> Self {
        Self {
            check_passes: review.check_passes,
        }
    }
}

/// Why the human is asked about the issues of a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskReason {
    /// Every issue is one that only the human may allow.
    OnlyTheHumanMayAllow,
    /// The team fixed nothing in the passes the checks allow: the human decides what the issues are worth.
    PassesUsed,
}

/// What the checking phase of a round comes to, and what the daemon does about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksStep {
    /// Nothing is left to hold the round back: the reviews start.
    StartReviews,
    /// The round goes back to the team with the reasons: this is the pass that starts.
    BackToTeam {
        /// Which pass of the checks this is, counted from 1.
        pass: NonZeroU32,
    },
    /// The round waits on a question to the human that names each issue; their answer starts the reviews or sends the
    /// round back.
    AskHuman {
        /// Why the human is asked.
        reason: AskReason,
    },
}

/// The checking phase of a round: after the gate is green at the head and before the reviews are asked, the first time
/// and after every fix pass.
///
/// It is not a state of [`RoundState`]: a round enters it from [`RoundState::Publishing`] and leaves it into
/// [`RoundState::InReview`] when [`Checking::conclude`] says [`ChecksStep::StartReviews`]; what it holds is how often
/// the checks have sent the round back. The daemon keeps that count with the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checking {
    /// How many times the checks have sent the round back to the team.
    passes: u32,
}

impl Checking {
    /// The checking phase of a round that the checks have sent back `passes` times.
    #[must_use]
    pub const fn after(passes: u32) -> Self {
        Self { passes }
    }

    /// How many times the checks have sent the round back to the team.
    #[must_use]
    pub const fn passes(&self) -> u32 {
        self.passes
    }

    /// What the `issues` of the round come to; `refused` are the subjects the human refused to allow since the last
    /// look, which go back to the team whatever the passes are.
    ///
    /// - No issue: the reviews start.
    /// - An issue the human refused: back to the team.
    /// - An issue the team can fix: back to the team while the policy has passes left, then the human is asked.
    /// - Only issues that the human alone may allow: the human is asked.
    #[must_use]
    pub fn conclude(&self, issues: &[Issue], refused: &[Subject], policy: ChecksPolicy) -> ChecksStep {
        if issues.is_empty() {
            return ChecksStep::StartReviews;
        }
        let back = ChecksStep::BackToTeam {
            pass: NonZeroU32::MIN.saturating_add(self.passes),
        };
        if issues.iter().any(|issue| refused.contains(&issue.subject())) {
            return back;
        }
        let team_can_fix = issues.iter().any(|issue| issue.judge() == Judge::Team);
        match (team_can_fix, self.passes < policy.check_passes) {
            (true, true) => back,
            (true, false) => ChecksStep::AskHuman {
                reason: AskReason::PassesUsed,
            },
            (false, _) => ChecksStep::AskHuman {
                reason: AskReason::OnlyTheHumanMayAllow,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    const AUTOPILOT: RoundPolicy = RoundPolicy {
        merge: MergeMode::Autopilot,
        fix_passes: 5,
    };

    const NO_MERGE: RoundPolicy = RoundPolicy {
        merge: MergeMode::NoMerge,
        fix_passes: 5,
    };

    /// A policy that asks the human at the first "changes requested".
    const ASK_AT_ONCE: RoundPolicy = RoundPolicy {
        merge: MergeMode::Autopilot,
        fix_passes: 0,
    };

    fn pr(n: u32) -> PrNumber {
        PrNumber::new(NonZeroU32::new(n).unwrap())
    }

    fn pass(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    /// A commit ID of forty copies of `digit`.
    fn head(digit: char) -> CommitId {
        digit.to_string().repeat(40).parse().unwrap()
    }

    fn reviewer(name: &str) -> InvestigatorName {
        name.parse().unwrap()
    }

    fn fixing(n: u32) -> Stage {
        Stage::Fixing {
            pr: pr(7),
            pass: pass(n),
        }
    }

    fn review(name: &str, verdict: ReviewVerdict) -> RoundEvent {
        RoundEvent::ReviewSubmitted {
            reviewer: reviewer(name),
            verdict,
        }
    }

    fn in_review(passes: u32, approvals: &[&str]) -> RoundState {
        RoundState::InReview {
            pr: pr(7),
            passes,
            approvals: approvals.iter().map(|name| reviewer(name)).collect(),
        }
    }

    fn step(state: RoundState, action: RoundAction) -> Step {
        Step { state, action }
    }

    /// The step `event` takes `state` through under `policy`.
    fn go(state: &RoundState, event: &RoundEvent, policy: RoundPolicy) -> Result<Step, RoundError> {
        state.transition(event, policy)
    }

    /// Feeds `events` one after the other from `state` and returns every step.
    fn run(mut state: RoundState, events: &[RoundEvent], policy: RoundPolicy) -> Vec<Step> {
        let mut steps = Vec::new();
        for event in events {
            let next = go(&state, event, policy).unwrap();
            state = next.state.clone();
            steps.push(next);
        }
        steps
    }

    // Rule: a round starts with the assigned member's work.
    #[test]
    fn a_round_starts_with_the_members_work() {
        assert_eq!(RoundState::start(), RoundState::MemberWork { stage: Stage::First });
    }

    // Rule: member work → coordinator integration.
    #[test]
    fn the_members_done_hands_its_branch_to_the_coordinator() {
        for stage in [Stage::First, fixing(1)] {
            assert_eq!(
                go(&RoundState::MemberWork { stage }, &RoundEvent::MemberDone, AUTOPILOT),
                Ok(step(RoundState::Integration { stage }, RoundAction::IntegrateBranch))
            );
        }
    }

    // Rule: integration → gate green at the head → the pull request is published for that head.
    #[test]
    fn a_green_gate_publishes_the_pull_request() {
        for stage in [Stage::First, fixing(2)] {
            assert_eq!(
                go(
                    &RoundState::Integration { stage },
                    &RoundEvent::GateGreen { head: head('a') },
                    AUTOPILOT
                ),
                Ok(step(
                    RoundState::Publishing { stage, head: head('a') },
                    RoundAction::PublishPullRequest
                ))
            );
        }
    }

    // Rule: a red gate keeps the round in integration and sends the coordinator back to fix it.
    #[test]
    fn a_red_gate_goes_back_to_the_coordinator() {
        for stage in [Stage::First, fixing(1)] {
            let state = RoundState::Integration { stage };
            assert_eq!(
                go(&state, &RoundEvent::GateFailed, AUTOPILOT),
                Ok(step(state, RoundAction::FixGate))
            );
        }
    }

    // Rule: the pull request is opened for the head the gate passed on; then two reviews are asked for.
    #[test]
    fn a_published_pull_request_asks_for_reviews() {
        let published = RoundEvent::PullRequestPublished {
            pr: pr(7),
            head: head('a'),
        };
        assert_eq!(
            go(
                &RoundState::Publishing {
                    stage: Stage::First,
                    head: head('a')
                },
                &published,
                AUTOPILOT
            ),
            Ok(step(in_review(0, &[]), RoundAction::RequestReviews))
        );
    }

    // Rule: a fix pass updates the same pull request and is reviewed again by two fresh reviews.
    #[test]
    fn an_updated_pull_request_is_reviewed_again() {
        let published = RoundEvent::PullRequestPublished {
            pr: pr(7),
            head: head('b'),
        };
        assert_eq!(
            go(
                &RoundState::Publishing {
                    stage: fixing(2),
                    head: head('b')
                },
                &published,
                AUTOPILOT
            ),
            Ok(step(in_review(2, &[]), RoundAction::RequestReviews))
        );
    }

    // Rule: a pull request that carries another commit than the one the gate passed on is refused.
    #[test]
    fn a_pull_request_for_another_head_than_the_gated_one_is_refused() {
        let published = RoundEvent::PullRequestPublished {
            pr: pr(7),
            head: head('c'),
        };
        assert_eq!(
            go(
                &RoundState::Publishing {
                    stage: Stage::First,
                    head: head('a')
                },
                &published,
                AUTOPILOT
            ),
            Err(RoundError::HeadChanged {
                gated: head('a'),
                published: head('c')
            })
        );
    }

    // Rule: a fix pass goes into the pull request that was reviewed: another one is refused.
    #[test]
    fn a_fix_pass_published_to_another_pull_request_is_refused() {
        let published = RoundEvent::PullRequestPublished {
            pr: pr(8),
            head: head('b'),
        };
        assert_eq!(
            go(
                &RoundState::Publishing {
                    stage: fixing(1),
                    head: head('b')
                },
                &published,
                AUTOPILOT
            ),
            Err(RoundError::OtherPullRequest {
                open: pr(7),
                other: pr(8)
            })
        );
    }

    // Rule: one approval is not enough: the round waits for the second review.
    #[test]
    fn the_first_approval_waits_for_the_second() {
        assert_eq!(
            go(
                &in_review(0, &[]),
                &review("inv-opus", ReviewVerdict::Approve),
                AUTOPILOT
            ),
            Ok(step(in_review(0, &["inv-opus"]), RoundAction::AwaitReviews))
        );
    }

    // Rule: two approvals from two investigators approve the item; autopilot merges.
    #[test]
    fn two_approvals_are_merged_in_autopilot() {
        assert_eq!(
            go(
                &in_review(0, &["inv-opus"]),
                &review("inv-codex", ReviewVerdict::Approve),
                AUTOPILOT
            ),
            Ok(step(RoundState::Approved { pr: pr(7) }, RoundAction::Merge))
        );
    }

    // Rule: with --no-merge the approved pull request is handed to the human, not merged.
    #[test]
    fn two_approvals_are_handed_to_the_human_with_no_merge() {
        assert_eq!(
            go(
                &in_review(0, &["inv-opus"]),
                &review("inv-codex", ReviewVerdict::Approve),
                NO_MERGE
            ),
            Ok(step(RoundState::Approved { pr: pr(7) }, RoundAction::HandToHuman))
        );
    }

    // Rule: both reviews must come from different investigators.
    #[test]
    fn the_same_reviewer_cannot_approve_twice() {
        assert_eq!(
            go(
                &in_review(0, &["inv-opus"]),
                &review("inv-opus", ReviewVerdict::Approve),
                AUTOPILOT
            ),
            Err(RoundError::DuplicateReview {
                reviewer: reviewer("inv-opus")
            })
        );
    }

    // Rule: a merge, by the harness or by the human, completes the item.
    #[test]
    fn a_merge_completes_the_item() {
        for policy in [AUTOPILOT, NO_MERGE] {
            assert_eq!(
                go(
                    &RoundState::Approved { pr: pr(7) },
                    &RoundEvent::Merged { pr: pr(7) },
                    policy
                ),
                Ok(step(RoundState::Done { pr: pr(7) }, RoundAction::CompleteItem))
            );
        }
    }

    // Rule: a merge of another pull request is refused.
    #[test]
    fn the_merge_of_another_pull_request_is_refused() {
        assert_eq!(
            go(
                &RoundState::Approved { pr: pr(7) },
                &RoundEvent::Merged { pr: pr(9) },
                AUTOPILOT
            ),
            Err(RoundError::OtherPullRequest {
                open: pr(7),
                other: pr(9)
            })
        );
    }

    // Rule: "changes requested" goes back to the team for a fix pass, counted from 1.
    #[test]
    fn changes_requested_start_a_fix_pass() {
        assert_eq!(
            go(
                &in_review(0, &[]),
                &review("inv-opus", ReviewVerdict::RequestChanges),
                AUTOPILOT
            ),
            Ok(step(
                RoundState::MemberWork { stage: fixing(1) },
                RoundAction::StartFixPass { pass: pass(1) }
            ))
        );
    }

    // Rule: one reviewer asking for changes decides, even after the other approved; the approval is dropped with
    // the round and the next review pass starts from none.
    #[test]
    fn changes_requested_after_an_approval_start_a_fix_pass() {
        assert_eq!(
            go(
                &in_review(1, &["inv-opus"]),
                &review("inv-codex", ReviewVerdict::RequestChanges),
                AUTOPILOT
            ),
            Ok(step(
                RoundState::MemberWork { stage: fixing(2) },
                RoundAction::StartFixPass { pass: pass(2) }
            ))
        );
    }

    // Rule: at most the configured number of fix passes: the last one is still started.
    #[test]
    fn the_last_configured_fix_pass_is_started() {
        assert_eq!(
            go(
                &in_review(4, &[]),
                &review("inv-opus", ReviewVerdict::RequestChanges),
                AUTOPILOT
            ),
            Ok(step(
                RoundState::MemberWork { stage: fixing(5) },
                RoundAction::StartFixPass { pass: pass(5) }
            ))
        );
    }

    // Rule: after the configured number of fix passes the human is asked.
    #[test]
    fn changes_requested_after_the_last_fix_pass_ask_the_human() {
        assert_eq!(
            go(
                &in_review(5, &[]),
                &review("inv-opus", ReviewVerdict::RequestChanges),
                AUTOPILOT
            ),
            Ok(step(
                RoundState::AskingHuman { pr: pr(7), passes: 5 },
                RoundAction::AskHuman
            ))
        );
    }

    // Rule: with no fix passes configured the human is asked at once.
    #[test]
    fn with_no_fix_passes_the_human_is_asked_at_once() {
        assert_eq!(
            go(
                &in_review(0, &[]),
                &review("inv-opus", ReviewVerdict::RequestChanges),
                ASK_AT_ONCE
            ),
            Ok(step(
                RoundState::AskingHuman { pr: pr(7), passes: 0 },
                RoundAction::AskHuman
            ))
        );
    }

    // Rule: the human's answer starts one more fix pass, which counts as a pass.
    #[test]
    fn the_humans_answer_starts_one_more_fix_pass() {
        assert_eq!(
            go(
                &RoundState::AskingHuman { pr: pr(7), passes: 5 },
                &RoundEvent::HumanAnswered,
                AUTOPILOT
            ),
            Ok(step(
                RoundState::MemberWork { stage: fixing(6) },
                RoundAction::StartFixPass { pass: pass(6) }
            ))
        );
    }

    // Rule: after the human's pass the next "changes requested" asks the human again.
    #[test]
    fn the_human_is_asked_again_after_the_granted_pass() {
        let steps = run(
            RoundState::AskingHuman { pr: pr(7), passes: 5 },
            &[
                RoundEvent::HumanAnswered,
                RoundEvent::MemberDone,
                RoundEvent::GateGreen { head: head('d') },
                RoundEvent::PullRequestPublished {
                    pr: pr(7),
                    head: head('d'),
                },
                review("inv-opus", ReviewVerdict::RequestChanges),
            ],
            AUTOPILOT,
        );
        assert_eq!(
            steps.last(),
            Some(&step(
                RoundState::AskingHuman { pr: pr(7), passes: 6 },
                RoundAction::AskHuman
            ))
        );
    }

    // Rule: the whole autopilot round, from the assignment to the item done.
    #[test]
    fn an_autopilot_round_runs_from_work_to_done() {
        let steps = run(
            RoundState::start(),
            &[
                RoundEvent::MemberDone,
                RoundEvent::GateFailed,
                RoundEvent::GateGreen { head: head('a') },
                RoundEvent::PullRequestPublished {
                    pr: pr(7),
                    head: head('a'),
                },
                review("inv-opus", ReviewVerdict::Approve),
                review("inv-codex", ReviewVerdict::Approve),
                RoundEvent::Merged { pr: pr(7) },
            ],
            AUTOPILOT,
        );
        let actions: Vec<RoundAction> = steps.iter().map(|s| s.action).collect();
        assert_eq!(
            actions,
            vec![
                RoundAction::IntegrateBranch,
                RoundAction::FixGate,
                RoundAction::PublishPullRequest,
                RoundAction::RequestReviews,
                RoundAction::AwaitReviews,
                RoundAction::Merge,
                RoundAction::CompleteItem,
            ]
        );
        assert_eq!(steps.last().map(|s| &s.state), Some(&RoundState::Done { pr: pr(7) }));
    }

    // Rule: the whole round with --no-merge hands the pull request to the human instead of merging it.
    #[test]
    fn a_no_merge_round_stops_at_the_human() {
        let steps = run(
            RoundState::start(),
            &[
                RoundEvent::MemberDone,
                RoundEvent::GateGreen { head: head('a') },
                RoundEvent::PullRequestPublished {
                    pr: pr(7),
                    head: head('a'),
                },
                review("inv-opus", ReviewVerdict::Approve),
                review("inv-codex", ReviewVerdict::Approve),
            ],
            NO_MERGE,
        );
        assert_eq!(
            steps.last(),
            Some(&step(RoundState::Approved { pr: pr(7) }, RoundAction::HandToHuman))
        );
        assert!(steps.iter().all(|s| s.action != RoundAction::Merge));
    }

    // Rule: a round with a fix pass: changes requested, the team fixes, the coordinator integrates, the pull request
    // is updated, and two new reviews approve it.
    #[test]
    fn a_round_with_a_fix_pass_ends_done() {
        let steps = run(
            in_review(0, &["inv-opus"]),
            &[
                review("inv-codex", ReviewVerdict::RequestChanges),
                RoundEvent::MemberDone,
                RoundEvent::GateGreen { head: head('b') },
                RoundEvent::PullRequestPublished {
                    pr: pr(7),
                    head: head('b'),
                },
                review("inv-opus", ReviewVerdict::Approve),
                review("inv-codex", ReviewVerdict::Approve),
                RoundEvent::Merged { pr: pr(7) },
            ],
            AUTOPILOT,
        );
        assert_eq!(
            steps.last(),
            Some(&step(RoundState::Done { pr: pr(7) }, RoundAction::CompleteItem))
        );
        assert_eq!(steps.get(3).map(|s| &s.state), Some(&in_review(1, &[])));
    }

    /// One state of each phase, and of both stages where a phase has them.
    fn states() -> Vec<RoundState> {
        vec![
            RoundState::MemberWork { stage: Stage::First },
            RoundState::MemberWork { stage: fixing(1) },
            RoundState::Integration { stage: Stage::First },
            RoundState::Integration { stage: fixing(1) },
            RoundState::Publishing {
                stage: Stage::First,
                head: head('a'),
            },
            RoundState::Publishing {
                stage: fixing(1),
                head: head('a'),
            },
            in_review(1, &["inv-opus"]),
            RoundState::Approved { pr: pr(7) },
            RoundState::AskingHuman { pr: pr(7), passes: 5 },
            RoundState::Done { pr: pr(7) },
        ]
    }

    /// One event of each kind, consistent with the states of [`states`].
    fn events() -> Vec<RoundEvent> {
        vec![
            RoundEvent::MemberDone,
            RoundEvent::GateGreen { head: head('a') },
            RoundEvent::GateFailed,
            RoundEvent::PullRequestPublished {
                pr: pr(7),
                head: head('a'),
            },
            review("inv-codex", ReviewVerdict::Approve),
            review("inv-codex", ReviewVerdict::RequestChanges),
            RoundEvent::Merged { pr: pr(7) },
            RoundEvent::HumanAnswered,
        ]
    }

    /// The phases and events that fit together; every other pair is [`RoundError::NotExpected`].
    const FITTING: &[(RoundPhase, RoundEventKind)] = &[
        (RoundPhase::MemberWork, RoundEventKind::MemberDone),
        (RoundPhase::Integration, RoundEventKind::GateGreen),
        (RoundPhase::Integration, RoundEventKind::GateFailed),
        (RoundPhase::Publishing, RoundEventKind::PullRequestPublished),
        (RoundPhase::InReview, RoundEventKind::ReviewSubmitted),
        (RoundPhase::Approved, RoundEventKind::Merged),
        (RoundPhase::AskingHuman, RoundEventKind::HumanAnswered),
    ];

    fn kind(event: &RoundEvent) -> RoundEventKind {
        match event {
            RoundEvent::MemberDone => RoundEventKind::MemberDone,
            RoundEvent::GateGreen { .. } => RoundEventKind::GateGreen,
            RoundEvent::GateFailed => RoundEventKind::GateFailed,
            RoundEvent::PullRequestPublished { .. } => RoundEventKind::PullRequestPublished,
            RoundEvent::ReviewSubmitted { .. } => RoundEventKind::ReviewSubmitted,
            RoundEvent::Merged { .. } => RoundEventKind::Merged,
            RoundEvent::HumanAnswered => RoundEventKind::HumanAnswered,
        }
    }

    // Rule: every event that does not fit the phase is refused with the phase and the event, and every one that fits is
    // not: a finished round takes no event at all.
    #[test]
    fn only_the_events_that_fit_a_phase_are_taken() {
        for state in states() {
            for event in events() {
                let fits = FITTING.contains(&(state.phase(), kind(&event)));
                let result = go(&state, &event, AUTOPILOT);
                if fits {
                    assert!(
                        !matches!(result, Err(RoundError::NotExpected { .. })),
                        "{state:?} refuses {event:?}"
                    );
                } else {
                    assert_eq!(
                        result,
                        Err(RoundError::NotExpected {
                            phase: state.phase(),
                            event: kind(&event)
                        }),
                        "{state:?} takes {event:?}"
                    );
                }
            }
        }
    }

    // Rule: every phase and every event kind is covered by the checks above.
    #[test]
    fn the_checks_cover_every_phase_and_event_kind() {
        let phases: Vec<RoundPhase> = states().iter().map(RoundState::phase).collect();
        for phase in RoundPhase::iter() {
            assert!(phases.contains(&phase), "no state of {phase}");
        }
        let kinds: Vec<RoundEventKind> = events().iter().map(kind).collect();
        for event in RoundEventKind::iter() {
            assert!(kinds.contains(&event), "no event of {event}");
        }
    }

    // Rule: the round never leaves the item in a phase its status table does not allow.
    #[test]
    fn every_transition_is_a_change_the_item_phase_allows() {
        for state in states() {
            for event in events() {
                let Ok(next) = go(&state, &event, AUTOPILOT) else {
                    continue;
                };
                let (from, to) = (state.item_phase(), next.state.item_phase());
                assert!(
                    from == to || from.successors().contains(&to),
                    "{state:?} --{event:?}--> {:?}: item phase {from} to {to}",
                    next.state
                );
            }
        }
    }

    // Rule: the item phase the round shows.
    #[test]
    fn the_item_phase_follows_the_round() {
        let expected = [
            (RoundState::MemberWork { stage: Stage::First }, PhaseKind::InProgress),
            (RoundState::MemberWork { stage: fixing(1) }, PhaseKind::Fixing),
            (RoundState::Integration { stage: Stage::First }, PhaseKind::InProgress),
            (RoundState::Integration { stage: fixing(1) }, PhaseKind::Fixing),
            (
                RoundState::Publishing {
                    stage: Stage::First,
                    head: head('a'),
                },
                PhaseKind::InProgress,
            ),
            (
                RoundState::Publishing {
                    stage: fixing(1),
                    head: head('a'),
                },
                PhaseKind::Fixing,
            ),
            (in_review(0, &[]), PhaseKind::InReview),
            (RoundState::AskingHuman { pr: pr(7), passes: 5 }, PhaseKind::InReview),
            (RoundState::Approved { pr: pr(7) }, PhaseKind::Approved),
            (RoundState::Done { pr: pr(7) }, PhaseKind::Done),
        ];
        for (state, phase) in expected {
            assert_eq!(state.item_phase(), phase, "{state:?}");
        }
    }

    // Rule: the policy takes its fix passes from the configuration's [review] table.
    #[test]
    fn the_policy_takes_the_fix_passes_of_the_review_configuration() {
        let config = crate::config::Config::from_toml_str("[review]\nfix_passes = 2\n").unwrap();
        assert_eq!(
            RoundPolicy::new(MergeMode::NoMerge, config.review()),
            RoundPolicy {
                merge: MergeMode::NoMerge,
                fix_passes: 2
            }
        );
        let default = crate::config::Config::default();
        assert_eq!(RoundPolicy::new(MergeMode::Autopilot, default.review()), AUTOPILOT);
    }
}
