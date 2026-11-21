//! Words of a task for the tests of the notifications.

use std::path::Path;

use agent_core::config::Language;
use agent_core::ids::TaskId;
use agent_core::task_list::TaskList;
use agent_runtime::harness_prompts::HarnessPrompts;

use super::TaskWords;
use crate::fixtures::task;

/// The texts of the catalog, from the prompts of the repository; the load is asynchronous, so it runs on a runtime of
/// its own, which makes this usable inside a test that has one too.
pub(crate) fn texts() -> HarnessPrompts {
    std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(HarnessPrompts::load(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
                &Language::default(),
            ))
            .unwrap()
    })
    .join()
    .unwrap()
}

/// The words of the task `task_id` whose list is `list`, for a notification that concerns `item`.
pub(crate) fn words_for(list: &TaskList, item: Option<String>, task_id: &TaskId) -> TaskWords {
    TaskWords::rendered(list, item, task_id, &texts()).unwrap()
}

/// The words of task 1 whose list is `list`, for a notification that concerns `item`.
pub(crate) fn words_of(list: &TaskList, item: Option<String>) -> TaskWords {
    words_for(list, item, &task(1))
}

/// The words of a task whose brief is `Fix the typo in the README. It has one.`, with no item.
pub(crate) fn words() -> TaskWords {
    words_of(
        &TaskList::parse("Fix the typo in the README. It has one.\n").unwrap(),
        None,
    )
}
