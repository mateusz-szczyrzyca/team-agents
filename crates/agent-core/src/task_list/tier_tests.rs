//! Tier directives preserve existing item text and report physical line numbers.

use super::*;

#[test]
fn reads_requested_tier_and_preserves_item_sections() {
    for (directive, expected) in [
        ("Tier: investigator\n", Tier::Investigator),
        ("Tier: normal\n", Tier::Normal),
        ("Tier:\tworker \n", Tier::Worker),
        ("", Tier::Normal),
    ] {
        let text = format!("1. Feature\n[ ] 1.1 Item\n{directive}    Café.\n    Checks:\n    Read it.\n");
        let actual = TaskList::parse(&text).map(|list| {
            let item = list.items().next().expect("one item");
            (
                list.required_tier(item.id),
                item.description.clone(),
                item.checks.clone(),
            )
        });
        assert_eq!(
            actual,
            Ok((Some(expected), "Café.".to_owned(), Some("Read it.".to_owned())))
        );
    }
}

#[test]
fn refuses_unknown_or_empty_tier_at_its_line() {
    for value in ["helper", "Investigator", "", "调查者", "normal worker"] {
        let error = TaskList::parse(&format!(
            "\u{feff}Brief.\r\n\r\n1. Feature\r\n[ ] 1.1 Item\r\nTier: {value}\r\n"
        ));
        assert!(matches!(error, Err(TaskListError::InvalidTier { line, source })
            if line.get() == 5 && source == UnknownKeyword::of::<Tier>(value)));
    }
}

#[test]
fn refuses_repeated_tier_for_one_item() {
    for second in ["normal", "worker"] {
        let result = TaskList::parse(&format!("1. Feature\n[ ] 1.1 Item\nTier: normal\nTier: {second}\n"));
        assert!(matches!(result, Err(TaskListError::DuplicateTier { line }) if line.get() == 4));
    }
}

#[test]
fn tier_is_item_local_and_may_follow_description() {
    let actual = TaskList::parse("1. Feature\n[ ] 1.1 First\n    First.\nTier: worker\n[ ] 1.2 Second\n    Second.\n")
        .map(|list| {
            list.items()
                .map(|item| (item.description.clone(), list.required_tier(item.id)))
                .collect::<Vec<_>>()
        });
    assert_eq!(
        actual,
        Ok(vec![
            ("First.".to_owned(), Some(Tier::Worker)),
            ("Second.".to_owned(), Some(Tier::Normal))
        ])
    );
}

#[test]
fn legacy_list_keeps_indented_tier_text_and_unknown_item_has_no_requirement() {
    let list =
        TaskList::parse("Tier: a brief\n1. Feature\n[ ] 1.1 Item\n    Tier: ordinary text\n").expect("legacy list");
    assert_eq!(list.brief(), "Tier: a brief");
    assert_eq!(list.items().next().expect("item").description, "Tier: ordinary text");
    assert_eq!(list.required_tier("1.1".parse().expect("id")), Some(Tier::Normal));
    assert_eq!(list.required_tier("1.2".parse().expect("id")), None);
}

#[test]
fn tier_before_any_item_is_refused_as_before() {
    assert!(matches!(TaskList::parse("1. Feature\nTier: normal\n"),
        Err(TaskListError::UnexpectedLine { line }) if line.get() == 2));
}
