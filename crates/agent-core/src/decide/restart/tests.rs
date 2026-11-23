use super::*;

#[test]
fn restart_budget_reserves_one_ordinal_only_below_the_limit() {
    for (used, limit, expected) in [
        (0, 0, Decision::Exhausted),
        (0, 1, Decision::Restart(NonZeroU32::new(1).unwrap())),
        (1, 1, Decision::Exhausted),
        (2, 1, Decision::Exhausted),
        (1, 2, Decision::Restart(NonZeroU32::new(2).unwrap())),
        (
            u32::MAX - 1,
            u32::MAX,
            Decision::Restart(NonZeroU32::new(u32::MAX).unwrap()),
        ),
        (u32::MAX, u32::MAX, Decision::Exhausted),
        (u32::MAX, 0, Decision::Exhausted),
    ] {
        assert_eq!(
            decide(RestartCount(used), limit),
            expected,
            "used={used}, limit={limit}"
        );
    }
}
