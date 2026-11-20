//! Human-only findings take precedence over the team's first check pass.

use agent_core::decide::checks::{Issue, Subject, Touch};
use agent_core::decide::round::{AskReason, Checking, ChecksPolicy, ChecksStep};

#[test]
fn mixed_checks_ask_the_human_before_spending_a_team_pass() {
    let human = Issue::ProtectedChanged {
        path: "ci/gate.sh".parse().unwrap(),
        change: Touch::Modified,
    };
    let policy = ChecksPolicy { check_passes: 2 };
    for passes in [0, 1] {
        for issues in [vec![human.clone(), Issue::NoTests], vec![Issue::NoTests, human.clone()]] {
            assert_eq!(
                Checking::after(passes).conclude(&issues, &[], policy),
                ChecksStep::AskHuman {
                    reason: AskReason::OnlyTheHumanMayAllow
                },
                "passes={passes}, issues={issues:?}",
            );
        }
    }
    // A refused finding must still reach the team even when it also has work of its own.
    assert_eq!(
        Checking::after(0).conclude(
            &[human, Issue::NoTests],
            &[Subject::Path("ci/gate.sh".parse().unwrap())],
            policy
        ),
        ChecksStep::BackToTeam {
            pass: std::num::NonZeroU32::MIN
        },
    );
}
