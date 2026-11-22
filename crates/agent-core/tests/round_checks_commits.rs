//! What the checks of a round come to when it has findings about commits: the team cannot repair them, so they are
//! asked of the human at once, spend no pass of the team and are never sent back for.

#[cfg(test)]
mod round_checks_commits {
    use std::num::NonZeroU32;

    use agent_core::decide::checks::{Issue, Subject, Touch};
    use agent_core::decide::round::{AskReason, Checking, ChecksPolicy, ChecksStep, CommitsLeft};
    use agent_core::repo_path::RepoPath;

    const TWICE: ChecksPolicy = ChecksPolicy { check_passes: 2 };

    fn path(text: &str) -> RepoPath {
        text.parse().unwrap()
    }

    fn back(pass: u32) -> ChecksStep {
        ChecksStep::BackToTeam {
            pass: NonZeroU32::new(pass).unwrap(),
        }
    }

    fn ask(reason: AskReason) -> ChecksStep {
        ChecksStep::AskHuman { reason }
    }

    fn team() -> Issue {
        Issue::NoTests
    }

    fn human() -> Issue {
        Issue::ProtectedChanged {
            path: path("AGENTS.md"),
            change: Touch::Modified,
        }
    }

    fn to_ask(count: usize) -> CommitsLeft {
        CommitsLeft::asking(count)
    }

    struct Row {
        name: &'static str,
        passes: u32,
        issues: Vec<Issue>,
        refused: Vec<Subject>,
        commits: CommitsLeft,
        step: ChecksStep,
    }

    /// What each outcome of the decision is, as a table.
    fn rows() -> Vec<Row> {
        vec![
            Row {
                name: "nothing is left",
                passes: 0,
                issues: vec![],
                refused: vec![],
                commits: CommitsLeft::default(),
                step: ChecksStep::StartReviews,
            },
            Row {
                name: "only commits nobody answered: the human at once, no pass spent",
                passes: 0,
                issues: vec![],
                refused: vec![],
                commits: to_ask(3),
                step: ask(AskReason::OnlyTheHumanMayAllow),
            },
            Row {
                name: "commits nobody answered after the passes were used: still only the human's part",
                passes: 2,
                issues: vec![],
                refused: vec![],
                commits: to_ask(1),
                step: ask(AskReason::OnlyTheHumanMayAllow),
            },
            Row {
                name: "commits and an issue of the team with passes left: the human is asked first",
                passes: 0,
                issues: vec![team()],
                refused: vec![],
                commits: to_ask(1),
                step: ask(AskReason::OnlyTheHumanMayAllow),
            },
            Row {
                name: "commits and an issue of the team with the passes used: the human is asked about all",
                passes: 2,
                issues: vec![team()],
                refused: vec![],
                commits: to_ask(1),
                step: ask(AskReason::PassesUsed),
            },
            Row {
                name: "commits and an issue only the human may allow",
                passes: 0,
                issues: vec![human()],
                refused: vec![],
                commits: to_ask(2),
                step: ask(AskReason::OnlyTheHumanMayAllow),
            },
            Row {
                name: "a commit the human refused leaves nothing to ask and holds nothing back: the team cannot undo a commit",
                passes: 0,
                issues: vec![],
                refused: vec![],
                commits: CommitsLeft::default(),
                step: ChecksStep::StartReviews,
            },
            Row {
                name: "a commit the human refused after one pass spends no pass either",
                passes: 1,
                issues: vec![],
                refused: vec![],
                commits: CommitsLeft::default(),
                step: ChecksStep::StartReviews,
            },
            Row {
                name: "a refused commit and one still to ask: the human is asked, nobody is sent back",
                passes: 0,
                issues: vec![],
                refused: vec![],
                commits: to_ask(1),
                step: ask(AskReason::OnlyTheHumanMayAllow),
            },
        ]
    }

    /// The outcomes when no finding about a commit is left, or one is and the issues decide.
    fn rows_without_commits() -> Vec<Row> {
        let refused_path = Subject::Path(path("AGENTS.md"));
        vec![
            Row {
                name: "no commit, an issue of the team with passes left: back to the team",
                passes: 0,
                issues: vec![team()],
                refused: vec![],
                commits: CommitsLeft::default(),
                step: back(1),
            },
            Row {
                name: "no commit, an issue of the team with the passes used",
                passes: 2,
                issues: vec![team()],
                refused: vec![],
                commits: CommitsLeft::default(),
                step: ask(AskReason::PassesUsed),
            },
            Row {
                name: "no commit, an issue only the human may allow",
                passes: 0,
                issues: vec![human()],
                refused: vec![],
                commits: CommitsLeft::default(),
                step: ask(AskReason::OnlyTheHumanMayAllow),
            },
            Row {
                name: "an issue about a file the human refused goes back whatever else is left",
                passes: 1,
                issues: vec![human()],
                refused: vec![refused_path],
                commits: to_ask(1),
                step: back(2),
            },
        ]
    }

    #[test]
    fn what_the_checks_come_to_with_findings_about_commits() {
        for row in rows().into_iter().chain(rows_without_commits()) {
            let step = Checking::after(row.passes).conclude_with_commits(&row.issues, &row.refused, row.commits, TWICE);
            assert_eq!(step, row.step, "{}", row.name);
        }
    }
}
