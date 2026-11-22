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

#[test]
fn unchanged_done_step_without_judgement_is_judged_again() {
    let plan = parse_plan("A task.\n\n1. Stage\n[x] 1.1 First step\n    Check: exists a.txt\n").unwrap();
    let judged = steps_to_judge(&plan, Some(&plan), |_| None);
    let ids: Vec<ItemId> = judged.iter().map(|step| step.id()).collect();
    assert_eq!(ids, vec!["1.1".parse::<ItemId>().unwrap()]);
}

#[test]
fn refuses_gate_words_separated_by_whitespace() {
    for (command, prefix, expected) in [
        ("agent  gate", "agent gate", true),
        ("agent\tgate --locked", "agent gate", true),
        ("make gate", "make\tgate", true),
        ("agent gateway", "agent  gate", false),
        ("agent gate", " \t", false),
    ] {
        assert_eq!(refuse_command(command, &[prefix.to_owned()]), expected, "{command:?}");
    }
}

#[test]
fn previously_confirmed_reopened_steps_are_selected_again() {
    for mark in [' ', '>', '-'] {
        let previous = parse_plan(&format!("Work.\n\n1. Work\n[{mark}] 1.1 Write it\n    Reopened.\n")).unwrap();
        let current = parse_plan("Work.\n\n1. Work\n[x] 1.1 Write it\n    Reopened.\n").unwrap();
        let selected = steps_to_judge(&current, Some(&previous), |_| Some(JudgementState::Confirmed));
        assert_eq!(
            selected.iter().map(|step| step.id()).collect::<Vec<_>>(),
            vec!["1.1".parse::<ItemId>().unwrap()]
        );
    }
}

/// The check of the first step of a plan whose step names `check` (none when it names none).
fn plan_check_of(check: Option<&str>) -> Plan {
    let line = check.map_or(String::new(), |check| format!("    Check: {check}\n"));
    parse_plan(&format!("Finish it.\n\n1. Work\n[x] 1.1 Do it\n{line}")).unwrap()
}

#[test]
fn plan_check_table() {
    let refuse = vec!["agent gate".to_owned(), "make gate".to_owned()];
    let rows: Vec<(&str, Option<&str>, u64, CheckRun<'static>)> = vec![
        ("no check", None, 120, CheckRun::Inline),
        ("reviewer", Some("reviewer"), 120, CheckRun::Inline),
        ("exists", Some("exists ok.txt"), 120, CheckRun::Inline),
        ("command runs", Some("cargo test"), 120, CheckRun::Run("cargo test")),
        ("refused gate", Some("make gate"), 120, CheckRun::Refused),
        (
            "refused gate with flags",
            Some("agent gate --locked"),
            120,
            CheckRun::Refused,
        ),
        (
            "prefix in a longer word",
            Some("make gate-full"),
            120,
            CheckRun::Run("make gate-full"),
        ),
        ("checks switched off", Some("cargo test"), 0, CheckRun::Disabled),
        ("refused beats switched off", Some("make gate"), 0, CheckRun::Refused),
        (
            "exists is inline when switched off",
            Some("exists ok.txt"),
            0,
            CheckRun::Inline,
        ),
    ];
    for (name, check, seconds, expected) in rows {
        let plan = plan_check_of(check);
        let step = plan.steps().next().unwrap();
        assert_eq!(check_run(step.check(), seconds, &refuse), expected, "case: {name}");
    }
}

#[test]
fn plan_check_without_refused_prefixes_runs_gate_commands() {
    let plan = plan_check_of(Some("make gate"));
    let step = plan.steps().next().unwrap();
    assert_eq!(check_run(step.check(), 120, &[]), CheckRun::Run("make gate"));
}

#[test]
fn steps_to_run_table() {
    struct Row<'a> {
        name: &'a str,
        current: &'a Plan,
        last_plan: Option<&'a Plan>,
        state_1_1: Option<JudgementState>,
        seconds: u64,
        expected: Vec<(ItemId, CheckRun<'a>)>,
    }

    let item = |id: &str| id.parse::<ItemId>().unwrap();
    let refuse = vec!["agent gate".to_owned(), "make gate".to_owned()];
    let two_commands =
        parse_plan("Plan.\n\n1. Work\n[x] 1.1 First\n    Check: cargo test\n[x] 1.2 Second\n    Check: cargo clippy\n")
            .unwrap();
    let command_and_exists = parse_plan(
        "Plan.\n\n1. Work\n[x] 1.1 First\n    Check: cargo test\n[x] 1.2 Second\n    Check: exists ok.txt\n",
    )
    .unwrap();
    let refused_gate = parse_plan("Plan.\n\n1. Work\n[x] 1.1 First\n    Check: make gate\n").unwrap();
    let one_command = parse_plan("Plan.\n\n1. Work\n[x] 1.1 First\n    Check: cargo test\n").unwrap();

    let rows = [
        Row {
            name: "two steps with commands",
            current: &two_commands,
            last_plan: None,
            state_1_1: None,
            seconds: 120,
            expected: vec![
                (item("1.1"), CheckRun::Run("cargo test")),
                (item("1.2"), CheckRun::Run("cargo clippy")),
            ],
        },
        Row {
            name: "a command and an exists",
            current: &command_and_exists,
            last_plan: None,
            state_1_1: None,
            seconds: 120,
            expected: vec![
                (item("1.1"), CheckRun::Run("cargo test")),
                (item("1.2"), CheckRun::Inline),
            ],
        },
        Row {
            name: "a refused command",
            current: &refused_gate,
            last_plan: None,
            state_1_1: None,
            seconds: 120,
            expected: vec![(item("1.1"), CheckRun::Refused)],
        },
        Row {
            name: "check_seconds = 0",
            current: &one_command,
            last_plan: None,
            state_1_1: None,
            seconds: 0,
            expected: vec![(item("1.1"), CheckRun::Disabled)],
        },
        Row {
            name: "a step confirmed before and unchanged",
            current: &one_command,
            last_plan: Some(&one_command),
            state_1_1: Some(JudgementState::Confirmed),
            seconds: 120,
            expected: vec![],
        },
        Row {
            name: "a mismatch still done",
            current: &one_command,
            last_plan: Some(&one_command),
            state_1_1: Some(JudgementState::Mismatch),
            seconds: 120,
            expected: vec![(item("1.1"), CheckRun::Run("cargo test"))],
        },
    ];

    for row in rows {
        let actual = steps_to_run(
            row.current,
            row.last_plan,
            |id| (*id == item("1.1")).then_some(row.state_1_1).flatten(),
            row.seconds,
            &refuse,
        );
        assert_eq!(actual, row.expected, "case: {}", row.name);
    }
}
