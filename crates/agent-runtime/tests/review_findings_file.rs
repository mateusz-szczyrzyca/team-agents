//! The file a reviewer writes its findings to while it reviews: one for each pull request and pass of its own task.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::TaskId;
use agent_core::state::PrNumber;
use agent_runtime::store::Home;

fn task(text: &str) -> TaskId {
    text.parse().unwrap()
}

fn pr(number: u32) -> PrNumber {
    PrNumber::new(NonZeroU32::new(number).unwrap())
}

fn home() -> Home {
    Home::new(&AbsolutePath::from_str("/srv/agents").unwrap())
}

#[test]
fn the_findings_file_is_named_after_the_pull_request_and_the_pass_in_the_task_directory_of_the_reviewer() {
    let home = home();
    let reviewer = task("task-20261111-153601-k3x9");

    assert_eq!(
        home.review_findings_file(&reviewer, pr(7), 2),
        home.task_dir(&reviewer).join("review-pr7-pass2-findings.md")
    );
}

#[test]
fn two_passes_two_pull_requests_and_two_reviewers_have_files_of_their_own() {
    let home = home();
    let (first, second) = (task("task-20261111-153601-k3x9"), task("task-20261111-153602-k3x8"));
    let files: Vec<PathBuf> = [
        home.review_findings_file(&first, pr(7), 1),
        home.review_findings_file(&first, pr(7), 2),
        home.review_findings_file(&first, pr(8), 1),
        home.review_findings_file(&second, pr(7), 1),
    ]
    .into();

    for (index, file) in files.iter().enumerate() {
        assert!(files.iter().skip(index + 1).all(|other| other != file), "{file:?}");
    }
}
