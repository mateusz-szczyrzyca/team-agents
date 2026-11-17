//! The head of a posted review, commit linking, and review format.

use agent_core::state::{ItemPhase, MergeMode};

use super::fake_github::SLUG;
use super::rig::{Rig, git, investigator, normal};

/// An approving final report.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";
/// A requested fix followed by approval.
const R1_SCENARIO: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] see `HEAD` for issue,done:VERDICT: APPROVE\\nSUMMARY: fixed in `HEAD`";

/// Captures the existing log event used when commit linking cannot consult git.
#[derive(Clone, Default)]
struct LinkEvents(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl tracing::field::Visit for LinkEvents {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "event" {
            self.0.lock().unwrap().push(format!("{value:?}"));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LinkEvents {
    fn on_event(&self, event: &tracing::Event<'_>, _context: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() == "agent::daemon::owner::rounds" {
            event.record(&mut self.clone());
        }
    }
}

#[tokio::test]
async fn unavailable_commit_lookups_send_original_text_and_log_the_existing_git_event() {
    use std::os::unix::fs::PermissionsExt;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_runtime::git::{CommitIdentity, Git};
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;

    use crate::daemon::owner::fixture::{Daemon, pair_state};

    let daemon = Daemon::new(&pair_state(), None);
    let (owner, _sinks) = daemon.bare_owner().await;
    for script in [
        "exit 128",
        "printf 'https://example.com/not-github/repo\\n'",
        "case \"$1\" in remote) printf 'https://github.com/octo/example.git\\n';; *) exit 128;; esac",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("git");
        std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = Git::new(
            AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap(),
            CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
        )
        .with_env("PATH".parse().unwrap(), dir.path().to_str().unwrap());
        let events = LinkEvents::default();
        let subscriber = tracing_subscriber::registry().with(events.clone());
        let original = "SUMMARY: inspect `abcdef0` and deadbeef — café\r\n";

        let commit = "abcdef0123456789abcdef0123456789abcdef01".parse().unwrap();
        let linked = owner
            .link_commits(&git, &commit, original)
            .with_subscriber(subscriber)
            .await;

        assert_eq!(linked, original);
        assert_eq!(
            *events.0.lock().unwrap(),
            vec![format!("{:?}", agent_core::log::LogEvent::GitCallFailed.as_str())],
            "script: {script}"
        );
    }
}

#[tokio::test]
async fn tree_blobs_is_listed_once_for_many_paths() {
    use std::fmt::Write as _;
    use std::os::unix::fs::PermissionsExt;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git};

    use crate::daemon::owner::fixture::{Daemon, pair_state};

    let daemon = Daemon::new(&pair_state(), None);
    let (owner, _sinks) = daemon.bare_owner().await;

    let dir = tempfile::tempdir().unwrap();
    let log_file = dir.path().join("calls.log");
    let executable = dir.path().join("git");
    let script = format!(
        "#!/bin/sh\nsubcmd=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    -c) shift 2 ;;\n    -*) shift ;;\n    *) subcmd=\"$1\"; break ;;\n  esac\ndone\necho \"$subcmd\" >> \"{}\"\ncase \"$subcmd\" in\n  remote) printf 'https://github.com/octo/example.git\\n' ;;\n  ls-tree) printf '100644 blob abcdef0123456789abcdef0123456789abcdef01\\tfile1.txt\\0' ;;\n  *) exit 128 ;;\nesac\n",
        log_file.display()
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();

    let git = Git::new(
        AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap(),
        CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
    )
    .with_env("PATH".parse().unwrap(), dir.path().to_str().unwrap());

    let commit: CommitId = "abcdef0123456789abcdef0123456789abcdef01".parse().unwrap();
    let mut many_paths = String::new();
    for i in 1..=100 {
        write!(many_paths, "file{i}.txt:1 ").unwrap();
    }

    drop(owner.link_commits(&git, &commit, &many_paths).await);

    let calls = std::fs::read_to_string(&log_file).unwrap();
    let ls_tree_count = calls.lines().filter(|line| *line == "ls-tree").count();
    assert_eq!(
        ls_tree_count, 1,
        "ls-tree should be called exactly once; calls:\n{calls}"
    );
}

#[tokio::test]
async fn review_head_and_commit_links_in_reviews_pr_description_and_update_comment() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work-pushed"),
            investigator("inv-r1", R1_SCENARIO),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (_, tasks) = rig
        .run(&["claude-1"], super::rig::ONE, MergeMode::Autopilot, None)
        .await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let initial_head = git(&rig.origin, &["rev-parse", &format!("refs/heads/{branch}~1")]);
    let final_head = rig.remote_tip(&branch);

    // 1. PR description links its commit ID
    let pulls = rig.github.pulls();
    assert_eq!(pulls.len(), 1);
    let pr_body = &pulls[0].body;
    let short_initial = initial_head.get(..7).unwrap();
    let expected_pr_link = format!("[`{short_initial}`](https://github.com/{SLUG}/commit/{initial_head})");
    assert!(
        pr_body.contains(&expected_pr_link),
        "PR body missing commit link: {pr_body}"
    );

    // 2. Updated branch comment links its commit ID
    let comments = rig.github.comments(1);
    assert_eq!(comments.len(), 1);
    let expected_update_link =
        format!("The branch was updated to [`{final_head}`](https://github.com/{SLUG}/commit/{final_head})");
    assert!(
        comments[0].contains(&expected_update_link),
        "Update comment missing commit link: {}",
        comments[0]
    );

    // 3. Reviews: 4 reviews total (2 on pass 1, 2 on pass 2)
    let reviews = rig.github.reviews(1);
    assert_eq!(reviews.len(), 4);

    // Pass 1: inv-r1 requested changes
    let r1_pass1 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r1`") && r["body"].as_str().unwrap().contains("pass 1")
        })
        .unwrap();
    let body1 = r1_pass1["body"].as_str().unwrap();
    assert!(body1.starts_with("### Review by `inv-r1` (claude)\n## ❌ $\\color{#d1242f}{\\textbf{REQUEST CHANGES}}$\nReview pass 1 of commit [`"));
    assert!(body1.contains(&format!("https://github.com/{SLUG}/commit/{initial_head}")));
    assert!(body1.contains("\n---\n\nBLOCKERS\n1. [TDD-2] see"));
    assert!(!body1.contains("VERDICT: REQUEST_CHANGES"));
    // Commit in backticks in report is linked
    assert!(body1.contains(&format!(
        "[`{initial_head}`](https://github.com/{SLUG}/commit/{initial_head})"
    )));

    // Pass 1: inv-r2 approved
    let r2_pass1 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r2`") && r["body"].as_str().unwrap().contains("pass 1")
        })
        .unwrap();
    let body2 = r2_pass1["body"].as_str().unwrap();
    assert!(body2.starts_with(
        "### Review by `inv-r2` (claude)\n## ✅ $\\color{#1a7f37}{\\textbf{APPROVED}}$\nReview pass 1 of commit [`"
    ));
    assert!(body2.contains("\n---\n\nSUMMARY: fine"));
    assert!(!body2.contains("VERDICT: APPROVE"));

    // Pass 2: inv-r1 approved on second pass
    let r1_pass2 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r1`") && r["body"].as_str().unwrap().contains("pass 2")
        })
        .unwrap();
    let body3 = r1_pass2["body"].as_str().unwrap();
    assert!(body3.starts_with(
        "### Review by `inv-r1` (claude)\n## ✅ $\\color{#1a7f37}{\\textbf{APPROVED}}$\nReview pass 2 of commit [`"
    ));
    assert!(body3.contains(&format!("https://github.com/{SLUG}/commit/{final_head}")));
    assert!(body3.contains("\n---\n\nSUMMARY: fixed in"));
    assert!(!body3.contains("VERDICT: APPROVE"));
}

#[tokio::test]
async fn modeled_reviews_and_late_reviews_link_the_reported_commit_on_each_pass() {
    for late in [
        "late-commit:APPROVE,review-commit:APPROVE",
        "late-commit:REQUEST_CHANGES,review-commit:APPROVE",
    ] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", "work"),
                investigator("inv-r1", "review-commit:REQUEST_CHANGES,review-commit:APPROVE"),
                investigator("inv-r2", late),
            ],
            5,
            false,
        )
        .await;
        let config_path = rig.home.config_file();
        let config = std::fs::read_to_string(&config_path).unwrap();
        let config = config
            .replace(
                "[investigators.inv-r1]\n",
                "[investigators.inv-r1]\nmodel = \"review-model\"\n",
            )
            .replace(
                "[investigators.inv-r2]\n",
                "[investigators.inv-r2]\nmodel = \"review-model\"\n",
            );
        std::fs::write(config_path, config).unwrap();
        rig.start().await;
        rig.run(&["claude-1"], super::rig::ONE, MergeMode::Autopilot, None)
            .await;
        rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
            .await;
        rig.stop().await.unwrap();
        let reviews = rig.github.reviews(1);
        assert_eq!(reviews.len(), 4);
        for review in reviews {
            let body = review["body"].as_str().unwrap();
            let head = review["commit_id"].as_str().unwrap();
            let commit = format!("[`{head}`](https://github.com/{SLUG}/commit/{head})");
            assert!(body.contains("(claude, review-model)\n"), "{body}");
            assert!(
                body.contains(&format!("of commit {commit}\n\n---\n\nSUMMARY: commit {commit}")),
                "{body}"
            );
            assert!(!body.contains("VERDICT:"), "{body}");
            assert_eq!(review["event"], "COMMENT");
        }
    }
}

#[tokio::test]
async fn file_links_in_reviews_pr_description_and_update_comment() {
    const TASK_LIST: &str = "\
Build the thing.

1. First
[ ] 1.1 The first item
    Check README.md:10-20 and see work-1.txt for details.
    Expected result:
    - the first thing works
    Checks:
    - run the first thing
";

    const R1_SCENARIO_FILES: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] see README.md:42 for issue,done:VERDICT: APPROVE\\nSUMMARY: fixed in `README.md`";
    const APPROVING_FILES: &str = "done:VERDICT: APPROVE\\nSUMMARY: see README.md";

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work-pushed"),
            investigator("inv-r1", R1_SCENARIO_FILES),
            investigator("inv-r2", APPROVING_FILES),
        ],
        5,
        false,
    )
    .await;

    let catalog = rig.home.prompts_dir();
    copy_prompt_catalog(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &catalog,
    )
    .unwrap();
    std::fs::write(
        catalog.join("harness/pr/update.md"),
        "The branch was updated to `{{head}}` for README.md:1-5 after fix pass {{pass}} (the gate: {{gate}}).\n",
    )
    .unwrap();

    rig.start_with_prompts(&catalog).await;

    let (_, tasks) = rig.run(&["claude-1"], TASK_LIST, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let initial_head = git(&rig.origin, &["rev-parse", &format!("refs/heads/{branch}~1")]);
    let final_head = rig.remote_tip(&branch);

    let pulls = rig.github.pulls();
    assert_eq!(pulls.len(), 1);
    let pr_body = &pulls[0].body;

    let expected_readme_link =
        format!("[README.md:10-20](https://github.com/{SLUG}/blob/{initial_head}/README.md#L10-L20)");
    let expected_work_link = format!("[work-1.txt](https://github.com/{SLUG}/blob/{initial_head}/work-1.txt)");
    assert!(
        pr_body.contains(&expected_readme_link),
        "PR body missing README file link: {pr_body}"
    );
    assert!(
        pr_body.contains(&expected_work_link),
        "PR body missing work-1.txt file link: {pr_body}"
    );

    let comments = rig.github.comments(1);
    assert_eq!(comments.len(), 1);
    let expected_update_file_link =
        format!("[README.md:1-5](https://github.com/{SLUG}/blob/{final_head}/README.md#L1-L5)");
    assert!(
        comments[0].contains(&expected_update_file_link),
        "Update comment missing file link: {}",
        comments[0]
    );

    let reviews = rig.github.reviews(1);
    assert_eq!(reviews.len(), 4);

    let r1_pass1 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r1`") && r["body"].as_str().unwrap().contains("pass 1")
        })
        .unwrap();
    let body1 = r1_pass1["body"].as_str().unwrap();
    let expected_r1_p1_link = format!("[README.md:42](https://github.com/{SLUG}/blob/{initial_head}/README.md#L42)");
    assert!(
        body1.contains(&expected_r1_p1_link),
        "Review pass 1 missing file link: {body1}"
    );

    let r1_pass2 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r1`") && r["body"].as_str().unwrap().contains("pass 2")
        })
        .unwrap();
    let body3 = r1_pass2["body"].as_str().unwrap();
    let expected_r1_p2_link = format!("[`README.md`](https://github.com/{SLUG}/blob/{final_head}/README.md)");
    assert!(
        body3.contains(&expected_r1_p2_link),
        "Review pass 2 missing backticked file link: {body3}"
    );
}

/// Copies the real prompt catalog so a scenario can override a text without touching shared files.
fn copy_prompt_catalog(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_prompt_catalog(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
