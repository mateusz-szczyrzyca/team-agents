use super::*;

#[test]
fn a_session_is_reminded_at_the_percentage_of_its_bound() {
    // (context, bound, percent, told, whether the session is reminded).
    let table = [
        (119_999, Some(150_000), 80, false, false),
        (120_000, Some(150_000), 80, false, true),
        (200_000, None, 80, false, false),
        (150_000, Some(150_000), 0, false, false),
        (150_000, Some(150_000), 80, true, false),
        (100, Some(100), 100, false, true),
        (99, Some(100), 100, false, false),
    ];
    for (context, bound, percent, told, expected) in table {
        assert_eq!(
            context_due(context, bound, percent, told),
            expected,
            "context {context}, bound {bound:?}, percent {percent}, told {told}"
        );
    }
}

#[test]
fn the_product_of_a_huge_context_and_percent_does_not_overflow() {
    assert!(context_due(u64::MAX, Some(u64::MAX), 100, false));
    assert!(!context_due(u64::MAX - 1, Some(u64::MAX), 100, false));
}
