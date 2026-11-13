//! The checking phase of a round through the public API: what the issues of the checks come to, and what the daemon does
//! about them.

#[cfg(test)]
mod round_checks {
    use std::num::NonZeroU32;

    use agent_core::config::Config;
    use agent_core::decide::checks::{Issue, Judge, Subject, TestChange, Touch};
    use agent_core::decide::round::{AskReason, Checking, ChecksPolicy, ChecksStep};
    use agent_core::repo_path::RepoPath;

    fn path(text: &str) -> RepoPath {
        text.parse().unwrap()
    }

    fn pass(n: u32) -> ChecksStep {
        ChecksStep::BackToTeam {
            pass: NonZeroU32::new(n).unwrap(),
        }
    }

    const TWICE: ChecksPolicy = ChecksPolicy { check_passes: 2 };

    /// An issue that the team can fix.
    fn for_the_team() -> Issue {
        Issue::NoTests
    }

    /// An issue that only the human may allow.
    fn for_the_human() -> Issue {
        Issue::ProtectedChanged {
            path: path("AGENTS.md"),
            change: Touch::Modified,
        }
    }

    #[test]
    fn the_issues_used_here_are_the_judges_they_are_taken_for() {
        assert_eq!(for_the_team().judge(), Judge::Team);
        assert_eq!(for_the_human().judge(), Judge::Human);
    }

    // Rule: a round with no issue goes to the reviewers, however often the checks sent it back.
    #[test]
    fn a_round_with_no_issue_starts_its_reviews() {
        for passes in [0, 1, 2, 9] {
            assert_eq!(
                Checking::after(passes).conclude(&[], &[], TWICE),
                ChecksStep::StartReviews,
                "{passes}"
            );
        }
    }

    // Rule: an issue the team can fix sends the round back, at most `check_passes` times; then the human is asked.
    #[test]
    fn an_issue_the_team_can_fix_sends_the_round_back_while_passes_are_left() {
        let issues = [for_the_team()];

        assert_eq!(Checking::after(0).conclude(&issues, &[], TWICE), pass(1));
        assert_eq!(Checking::after(1).conclude(&issues, &[], TWICE), pass(2));
    }

    #[test]
    fn when_the_passes_are_used_the_human_is_asked() {
        let issues = [for_the_team()];

        for passes in [2, 3, u32::MAX] {
            assert_eq!(
                Checking::after(passes).conclude(&issues, &[], TWICE),
                ChecksStep::AskHuman {
                    reason: AskReason::PassesUsed
                },
                "{passes}"
            );
        }
    }

    #[test]
    fn with_no_passes_the_human_is_asked_at_once() {
        let never = ChecksPolicy { check_passes: 0 };

        assert_eq!(
            Checking::after(0).conclude(&[for_the_team()], &[], never),
            ChecksStep::AskHuman {
                reason: AskReason::PassesUsed
            }
        );
    }

    // Rule: what only the human may allow is asked at once, whatever the passes; the team cannot fix it by trying again.
    #[test]
    fn an_issue_only_the_human_may_allow_asks_the_human() {
        let issues = [for_the_human()];

        for passes in [0, 1, 5] {
            assert_eq!(
                Checking::after(passes).conclude(&issues, &[], TWICE),
                ChecksStep::AskHuman {
                    reason: AskReason::OnlyTheHumanMayAllow
                },
                "{passes}"
            );
        }
    }

    // Rule: with both kinds the team goes first: what it can fix is fixed before the human is bothered, and the human is
    // asked about everything when the passes are used.
    #[test]
    fn with_both_kinds_the_team_goes_first_and_the_human_is_asked_when_passes_are_used() {
        let issues = [for_the_human(), for_the_team()];

        assert_eq!(Checking::after(0).conclude(&issues, &[], TWICE), pass(1));
        assert_eq!(
            Checking::after(2).conclude(&issues, &[], TWICE),
            ChecksStep::AskHuman {
                reason: AskReason::PassesUsed
            }
        );
    }

    // Rule: what the human refused goes back to the team, even when only they could have allowed it and even when the
    // passes are used: the human decided.
    #[test]
    fn an_issue_the_human_refused_goes_back_to_the_team() {
        let issues = [for_the_human()];
        let refused = [Subject::Path(path("AGENTS.md"))];

        assert_eq!(Checking::after(0).conclude(&issues, &refused, TWICE), pass(1));
        assert_eq!(Checking::after(2).conclude(&issues, &refused, TWICE), pass(3));
    }

    #[test]
    fn a_refusal_of_something_that_is_no_longer_an_issue_changes_nothing() {
        let refused = [Subject::Path(path("AGENTS.md"))];

        assert_eq!(
            Checking::after(0).conclude(&[], &refused, TWICE),
            ChecksStep::StartReviews
        );
        // another subject is refused: the issue at hand is the human's to decide
        assert_eq!(
            Checking::after(0).conclude(
                &[Issue::TestChanged {
                    path: path("tests/a.rs"),
                    how: TestChange::Deleted
                }],
                &refused,
                TWICE
            ),
            ChecksStep::AskHuman {
                reason: AskReason::OnlyTheHumanMayAllow
            }
        );
    }

    #[test]
    fn the_pass_that_starts_never_overflows() {
        let step = Checking::after(u32::MAX).conclude(&[for_the_human()], &[Subject::Path(path("AGENTS.md"))], TWICE);

        assert_eq!(step, ChecksStep::BackToTeam { pass: NonZeroU32::MAX });
    }

    #[test]
    fn the_phase_says_how_often_the_round_was_sent_back() {
        assert_eq!(Checking::after(3).passes(), 3);
        assert_eq!(Checking::after(0).passes(), 0);
    }

    #[test]
    fn the_policy_takes_its_passes_from_the_review_configuration() {
        let config = Config::from_toml_str("[review]\ncheck_passes = 4\n").unwrap();
        let default = Config::default();

        assert_eq!(ChecksPolicy::new(config.review()), ChecksPolicy { check_passes: 4 });
        assert_eq!(ChecksPolicy::new(default.review()), TWICE);
    }
}
