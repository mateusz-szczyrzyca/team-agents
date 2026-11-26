//! Reviewed-head gate provenance and its absence.
use super::*;
use agent_core::config::Language;
use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
use std::path::Path;

#[tokio::test]
async fn reviewers_reuse_only_the_gate_of_the_reviewed_head() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let head: CommitId = "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap();
    let mut record = GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: head.clone(),
            finished_at: "2026-11-19T09:30:00Z".parse().unwrap(),
            outcome: GateOutcome::Passed,
            log: "/tmp/tasks/task-1/gate-2.log".parse().unwrap(),
        },
    };
    assert_eq!(
        review_gate(&texts, &head, Some(&record)).unwrap(),
        include_str!("gate-present.md").trim_end()
    );
    record.result.head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap();
    for record in [Some(&record), None] {
        assert_eq!(
            review_gate(&texts, &head, record).unwrap(),
            include_str!("gate-absent.md").trim_end()
        );
    }
}

#[tokio::test]
async fn review_gate_in_formats_finished_time_in_the_daemon_zone() {
    use agent_core::local_time::Zone;
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let head: CommitId = "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap();
    let record = GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: head.clone(),
            finished_at: "2026-11-19T09:30:00Z".parse().unwrap(),
            outcome: GateOutcome::Passed,
            log: "/tmp/tasks/task-1/gate-2.log".parse().unwrap(),
        },
    };
    let east = Zone::Fixed(time::UtcOffset::from_hms(1, 0, 0).expect("valid"));

    let rendered = review_gate_in(&texts, &head, Some(&record), east).unwrap();

    assert!(rendered.contains("Finished: 2026-11-19 10:30:00."), "{rendered}");
}

// Rule: the findings file is named only where it is: a pass begun before the file was kept, or one whose file could not be
// written, is not sent to a file that is not there.
#[tokio::test]
async fn the_findings_file_is_named_only_when_it_exists() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fix-pass-findings.md");

    let missing = findings_file_line(&texts, &file).unwrap();
    std::fs::write(&file, "findings").unwrap();
    let present = findings_file_line(&texts, &file).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let a_directory = findings_file_line(&texts, &file).unwrap();

    assert_eq!(missing, None);
    assert!(present.is_some_and(|line| line.contains(&file.display().to_string())));
    assert_eq!(a_directory, None);
}

/// The fields `event` and `error` of the events logged while it is installed.
#[derive(Clone, Default)]
struct Logged(std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

/// Collects the two fields of one event.
#[derive(Default)]
struct Fields {
    /// The text of the field `event`.
    event: String,
    /// The text of the field `error`.
    error: String,
}

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "event" => self.event = format!("{value:?}"),
            "error" => self.error = format!("{value:?}"),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "event" => self.event = value.to_owned(),
            "error" => self.error = value.to_owned(),
            _ => {}
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Logged {
    fn on_event(&self, event: &tracing::Event<'_>, _context: tracing_subscriber::layer::Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push((fields.event, fields.error));
    }
}

/// The prompt texts of the repository.
async fn repository_texts() -> AutopilotPrompts {
    AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

/// Git in `dir`, with an identity of no importance.
fn git_in(dir: &Path) -> Git {
    Git::new(
        AbsolutePath::from_str(dir.to_str().unwrap()).unwrap(),
        CommitIdentity::new("Example".to_owned(), "agent@example.com".to_owned(), None),
    )
}

// Rule: a branch that git cannot read is logged with git's answer and leaves the fix pass without the text about another
// branch, as the checkout before a run does: the pass is not refused for it.
#[tokio::test]
async fn an_unreadable_branch_gives_no_other_branch_text_and_is_logged() {
    use tracing_subscriber::layer::SubscriberExt;

    let texts = repository_texts().await;
    let not_a_repository = tempfile::tempdir().unwrap();
    let logged = Logged::default();
    let _installed = tracing::subscriber::set_default(tracing_subscriber::registry().with(logged.clone()));

    let text = other_branch_text(&texts, &git_in(not_a_repository.path()), &"item-1".parse().unwrap())
        .await
        .unwrap();

    assert_eq!(text, None);
    // The runtime logs its own record of the failed call, which has no `error` field.
    let told: Vec<_> = logged
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, error)| !error.is_empty())
        .cloned()
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].0, LogEvent::GitCallFailed.as_str());
    assert!(told[0].1.contains("current branch failed"), "{told:?}");
}

// Rule: a worktree that has another branch out is told so, and one that has the item's branch out is not.
#[tokio::test]
async fn the_other_branch_text_names_both_branches_only_for_another_branch() {
    let texts = repository_texts().await;
    let repository = tempfile::tempdir().unwrap();
    let git = git_in(repository.path());
    for args in [
        &["init", "-q", "-b", "other"][..],
        &[
            "-c",
            "user.name=Example",
            "-c",
            "user.email=agent@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "First",
        ][..],
    ] {
        let done = std::process::Command::new("git")
            .args(args)
            .current_dir(repository.path())
            .status()
            .unwrap();
        assert!(done.success());
    }

    let item: BranchName = "item-1".parse().unwrap();
    let told = other_branch_text(&texts, &git, &item).await.unwrap().unwrap();
    let own = other_branch_text(&texts, &git, &"other".parse().unwrap())
        .await
        .unwrap();

    assert!(told.contains("Your worktree has `other` out"), "{told}");
    assert!(told.contains("git switch item-1"), "{told}");
    assert_eq!(own, None);
}
