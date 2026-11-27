//! A task whose agent has another account than the session of its task began under starts a new session from the
//! briefing, as `agent renew` does, and never resumes the old session; the account of the session survives a restart.

use agent_core::config::AccountName;
use agent_core::decide::ladder::Trouble;
use agent_core::state::{ItemPhase, MergeMode, Renewals};

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The header every briefing of a new session starts with.
const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";

/// The script of the run that is cut: it commits a file, leaves another one uncommitted and a plan, and holds until the
/// daemon stops.
const CUT: &str = r#"
exclude=$(git rev-parse --git-path info/exclude)
grep -qxF '.fake-*' "$exclude" || printf '%s\n' '.fake-*' >> "$exclude"
printf 'one\n' > kept.txt
git add kept.txt
git -c user.name=Fixture -c user.email=fixture@example.com commit -q -m 'Add kept.txt'
printf 'three\n' > staged.txt
mkdir -p .agent
printf 'Do the work.\n\n1. Work\n[>] 1.1 Finish the thing\n    Check: reviewer\n' > .agent/plan.md
for _ in $(seq 1 600); do
  [[ -f .fake-continue ]] && break
  sleep 0.05
done
"#;

/// The script of a run that only holds until it is released or the daemon stops.
const HOLD: &str = r#"
exclude=$(git rev-parse --git-path info/exclude)
grep -qxF '.fake-*' "$exclude" || printf '%s\n' '.fake-*' >> "$exclude"
for _ in $(seq 1 600); do
  [[ -f .fake-continue ]] && break
  sleep 0.05
done
"#;

/// The script of the run that goes on: it commits what the worktree holds.
const FINISH: &str = r#"
exclude=$(git rev-parse --git-path info/exclude)
grep -qxF '.fake-*' "$exclude" || printf '%s\n' '.fake-*' >> "$exclude"
git add -A
git -c user.name=Fixture -c user.email=fixture@example.com commit -q -m 'Finish the work'
"#;

/// The account the agent of these tests gets.
const ACCOUNT: &str = "comp-claude";

/// A rig whose agent `comp-claude-1` runs `scenario` with `[restart] session = "same"`, so that a cut run is resumed in its
/// session unless something else asks for a new one, and reviewers that approve; the agent has no account yet.
async fn rig_with(scenario: &'static str) -> Rig {
    let rig = Rig::new(
        &[
            normal("comp-claude-1", scenario),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("cut.sh"), CUT).unwrap();
    std::fs::write(scripts.join("hold.sh"), HOLD).unwrap();
    std::fs::write(scripts.join("finish.sh"), FINISH).unwrap();
    let path = rig.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, format!("{text}\n[restart]\nsession = \"same\"\n")).unwrap();
    rig
}

/// Gives the agent `comp-claude-1` the account [`ACCOUNT`] in the configuration of `rig`: the next start of the daemon reads it.
fn give_the_account(rig: &Rig) {
    let path = rig.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[agents.comp-claude-1]\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"{ACCOUNT}\"\n"));
    std::fs::write(path, format!("{text}\n[accounts.{ACCOUNT}]\nowner = \"comp\"\n")).unwrap();
}

// Rule: the session began under no account; the agent has one when the daemon starts again: the cut run does not resume
// its session (`[restart] session = "same"` notwithstanding) but starts a new one whose prompt is the briefing with the
// reason `account_changed`, the commits, the uncommitted files and the plan; no renewal is counted and the round goes on.
#[tokio::test]
async fn account_changed_between_runs_starts_from_briefing() {
    let mut rig = rig_with("script:cut,script:finish").await;
    rig.start().await;
    let (_, tasks) = rig.run(&["comp-claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the cut run to leave its files", || {
        worktree.join("staged.txt").exists()
    })
    .await;

    rig.stop().await.unwrap();
    give_the_account(&rig);
    rig.start().await;
    let briefing = rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    let state = rig
        .item_where("1.1", "merged after the new session", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(briefing.contains("`account_changed`"), "{briefing}");
    assert!(briefing.contains("the account of its agent changed"), "{briefing}");
    assert!(briefing.contains("Add kept.txt"), "the commits: {briefing}");
    assert!(briefing.contains("- staged.txt"), "the uncommitted files: {briefing}");
    assert!(briefing.contains("Finish the thing"), "the plan: {briefing}");
    assert!(args.contains("--session-id"), "a new session: {args}");
    assert!(!args.contains("--resume"), "not the old session: {args}");
    let member = state.find_task(&tasks[0]).unwrap().member;
    let renewals = member.renewals().expect("the account began a new session");
    assert_eq!(renewals.reason(), Trouble::AccountChanged);
    assert_eq!(renewals.used(), 0, "no run went bad");
    assert_eq!(
        state.session_account(&tasks[0]),
        Some(&ACCOUNT.parse::<AccountName>().unwrap())
    );
    assert_ne!(member.runs()[0].session(), member.runs()[1].session());
}

// Rule: the account the session began under is in the state: when the daemon stops again in the new session and starts
// once more, with the same account, the cut run resumes that session and is not briefed a second time.
#[tokio::test]
async fn account_changed_between_runs_starts_from_briefing_across_a_daemon_restart() {
    let mut rig = rig_with("script:cut,script:hold,script:finish").await;
    rig.start().await;
    let (_, tasks) = rig.run(&["comp-claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the cut run to leave its files", || {
        worktree.join("staged.txt").exists()
    })
    .await;
    rig.stop().await.unwrap();
    give_the_account(&rig);
    rig.start().await;
    let first = rig.prompt(&tasks[0], 2).await;
    assert!(first.starts_with(BRIEFING), "{first}");

    rig.stop().await.unwrap();
    rig.start().await;
    let second = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-3")).unwrap();
    let state = rig
        .item_where("1.1", "merged after the restart", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(!second.contains(BRIEFING), "never a second briefing: {second}");
    assert!(args.contains("--resume"), "the session of the account: {args}");
    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(member.renewals().map(Renewals::reason), Some(Trouble::AccountChanged));
    assert_eq!(member.runs()[1].session(), member.runs()[2].session());
    assert_eq!(
        state.session_account(&tasks[0]),
        Some(&ACCOUNT.parse::<AccountName>().unwrap())
    );
}
