use std::collections::HashMap;

use super::*;
use crate::ids::ItemId;
use crate::plan::parse_plan;

#[test]
fn refuse_command_table() {
    let prefixes = vec!["agent gate".to_owned(), "make gate".to_owned()];

    let cases = [
        ("agent gate", true),
        ("make gate", true),
        ("  agent gate  ", true),
        ("\tmake gate\t", true),
        ("agent gate --locked", true),
        ("make gate-full", false),
        ("agent gateway", false),
        ("make gater", false),
        ("agent", false),
        ("make", false),
        ("cargo test", false),
        ("", false),
    ];

    for (command, expected) in cases {
        assert_eq!(refuse_command(command, &prefixes), expected, "command: {command:?}");
    }

    assert!(!refuse_command("agent gate", &[]));
}

#[test]
fn steps_to_judge_table() {
    let plan1_text = "\
A task.

1. Stage
[x] 1.1 First step
    Check: exists a.txt
[ ] 1.2 Second step
    Check: exists b.txt
[>] 1.3 Third step
    Check: exists c.txt
[-] 1.4 Fourth step
    Given up.
";
    let plan1 = parse_plan(plan1_text).unwrap();

    // In a first version of a plan (last is None), only done steps are judged.
    let judged1 = steps_to_judge(&plan1, None, |_| None);
    assert_eq!(judged1.len(), 1);
    assert_eq!(judged1[0].id(), "1.1".parse::<ItemId>().unwrap());

    // In plan2:
    // 1.1 is still done, confirmed, unchanged -> NOT judged
    // 1.2 is now done (newly done) -> judged
    // 1.3 was in hand, now done -> judged
    // 1.4 was given up, now done -> judged
    let plan2_text = "\
A task.

1. Stage
[x] 1.1 First step
    Check: exists a.txt
[x] 1.2 Second step
    Check: exists b.txt
[x] 1.3 Third step
    Check: exists c.txt
[x] 1.4 Fourth step
    Check: exists d.txt
";
    let plan2 = parse_plan(plan2_text).unwrap();
    let mut states1 = HashMap::new();
    states1.insert("1.1".parse::<ItemId>().unwrap(), JudgementState::Confirmed);

    let judged2 = steps_to_judge(&plan2, Some(&plan1), |id| states1.get(id).copied());
    let ids2: Vec<ItemId> = judged2.iter().map(|s| s.id()).collect();
    assert_eq!(
        ids2,
        vec![
            "1.2".parse::<ItemId>().unwrap(),
            "1.3".parse::<ItemId>().unwrap(),
            "1.4".parse::<ItemId>().unwrap(),
        ]
    );

    // If 1.1 had been a mismatch in plan1, it is judged again in plan2:
    let mut states_mismatch = HashMap::new();
    states_mismatch.insert("1.1".parse::<ItemId>().unwrap(), JudgementState::Mismatch);
    let judged_mismatch = steps_to_judge(&plan2, Some(&plan1), |id| states_mismatch.get(id).copied());
    let ids_mismatch: Vec<ItemId> = judged_mismatch.iter().map(|s| s.id()).collect();
    assert!(ids_mismatch.contains(&"1.1".parse::<ItemId>().unwrap()));

    // If 1.1 text changed in plan3 (e.g. check changed), it is judged again:
    let plan3_text = "\
A task.

1. Stage
[x] 1.1 First step
    Check: exists changed.txt
";
    let plan3 = parse_plan(plan3_text).unwrap();
    let judged_changed = steps_to_judge(&plan3, Some(&plan1), |id| states1.get(id).copied());
    assert_eq!(judged_changed.len(), 1);
    assert_eq!(judged_changed[0].id(), "1.1".parse::<ItemId>().unwrap());
}
