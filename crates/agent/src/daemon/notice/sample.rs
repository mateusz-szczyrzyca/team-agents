//! Words of a task for the tests of the notifications.

use agent_core::task_list::TaskList;

use super::TaskWords;

/// The words of a task whose brief is `Fix the typo in the README. It has one.`, with no item.
pub(crate) fn words() -> TaskWords {
    let list = TaskList::parse("Fix the typo in the README. It has one.\n").unwrap();
    TaskWords::new(&list, None)
}
