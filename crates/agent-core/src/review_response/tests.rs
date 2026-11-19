//! Fix-response syntax and one-shot explanation boundaries.
use super::*;

#[test]
fn accepts_fixed_blockers_and_one_major_explanation() {
    let open = [
        (NonZeroU32::MIN, Severity::Blocker, false),
        (NonZeroU32::new(2).unwrap(), Severity::Major, false),
    ];
    assert_eq!(
        parse_lead(
            "FINDING 1: FIXED\nFINDING 2: EXPLAINED — nécessaire\nSTATUS: DONE",
            &open
        )
        .unwrap(),
        vec![
            (open[0].0, Response::Fixed),
            (open[1].0, Response::Explained("nécessaire".into()))
        ]
    );
}

#[test]
fn refuses_missing_duplicate_unknown_and_forbidden_responses() {
    for (text, severity, used, line) in [
        ("STATUS: DONE", Severity::Major, false, 2),
        ("FINDING 1: EXPLAINED — why", Severity::Blocker, false, 1),
        ("FINDING 1: EXPLAINED — why", Severity::Major, true, 1),
        ("FINDING 1: SECOND OPINION — why", Severity::Blocker, false, 1),
        ("FINDING 1: EXPLAINED — ", Severity::Major, false, 1),
        ("FINDING 1: FIXED\nFINDING 1: FIXED", Severity::Major, false, 2),
        ("FINDING 2: FIXED", Severity::Major, false, 1),
        ("STATUS: DONE\nFINDING 1: FIXED", Severity::Major, false, 2),
    ] {
        let err = parse_lead(text, &[(NonZeroU32::MIN, severity, used)]).expect_err(text);
        assert_eq!((err.line, err.number), (line, NonZeroU32::MIN));
    }
    assert_eq!(parse_lead("STATUS: DONE", &[]).unwrap(), vec![]);
}
