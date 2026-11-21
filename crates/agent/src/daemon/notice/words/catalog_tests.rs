//! The ID line and the end of a notification are texts of the catalog, rendered from `prompts/`, not built in the code.
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::ids::TaskId;
use agent_core::kinds::CliKind;
use agent_core::task_list::TaskList;
use agent_runtime::harness_prompts::HarnessPrompts;

use super::*;
use crate::daemon::notice::{NoticeContext, NoticeKind, layout_notification};
use crate::fixtures::{at, task};

/// The directory of the prompts of the repository.
fn prompts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
}

/// The texts of the catalog loaded from `dir`; the load is asynchronous, so it runs on a runtime of its own.
fn load(dir: PathBuf) -> HarnessPrompts {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(HarnessPrompts::load(&dir, &Language::default()))
            .unwrap()
    })
    .join()
    .unwrap()
}

/// Copies the directory `from` to `to`, whole.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The notification that `words` make about `task_id`, with the note and the message `Note\nMessage.`.
fn notification_of(words: TaskWords, task_id: &TaskId) -> String {
    let context = NoticeContext {
        repo: "team-agents",
        agent: "claude-1",
        cli: Some(CliKind::Claude),
        model: None,
        time: at(0),
        zone: agent_core::local_time::Zone::Fixed(time::UtcOffset::UTC),
        task: task_id,
        team: None,
        pr: None,
        words,
    };
    layout_notification(NoticeKind::Failed, &context, "Note\nMessage.")
        .unwrap()
        .body()
        .to_owned()
}

#[test]
fn the_texts_of_the_catalog_give_the_id_line_and_the_end_of_a_notification() {
    let task_id = task(1);
    let list = TaskList::parse("Fix the typo. It has one.\n").unwrap();
    let words = TaskWords::rendered(&list, None, &task_id, &load(prompts_dir())).unwrap();

    let body = notification_of(words, &task_id);

    assert!(body.contains(&format!("\nTask ID: {task_id}\n")), "{body}");
    assert!(
        body.ends_with(&format!("\n\nTASK ({task_id}):\nFix the typo. It has one.")),
        "{body}"
    );
}

#[test]
fn another_text_of_the_catalog_gives_another_line_and_another_end() {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&prompts_dir(), dir.path());
    std::fs::write(
        dir.path().join("harness/run/notice-task.md"),
        "Zadanie nr {{task}}\nZADANIE ({{task}}) — {{excerpt}}\n",
    )
    .unwrap();
    let task_id = task(1);
    let list = TaskList::parse("Napraw literówkę. Jest jedna.\n").unwrap();
    let words = TaskWords::rendered(&list, None, &task_id, &load(dir.path().to_path_buf())).unwrap();

    let body = notification_of(words, &task_id);

    assert!(body.contains(&format!("\nZadanie nr {task_id}\n")), "{body}");
    assert!(!body.contains("Task ID:"), "{body}");
    assert!(
        body.ends_with(&format!("\n\nZADANIE ({task_id}) — Napraw literówkę. Jest jedna.")),
        "{body}"
    );
    assert!(!body.contains("TASK ("), "{body}");
}

#[test]
fn a_brief_without_words_has_no_end_whatever_the_text_says() {
    let task_id = task(1);
    let list = TaskList::parse("1. Feature\n[ ] 1.1 Item\n    Do it.\n").unwrap();
    let words = TaskWords::rendered(&list, None, &task_id, &load(prompts_dir())).unwrap();

    let body = notification_of(words, &task_id);

    assert!(!body.contains("TASK ("), "{body}");
    assert!(body.contains(&format!("\nTask ID: {task_id}\n")), "{body}");
}
