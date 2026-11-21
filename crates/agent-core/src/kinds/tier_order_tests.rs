//! The capability order is independent of the keyword declaration order.

use super::Tier;

#[test]
fn tier_comparisons_follow_capability_order() {
    let ordered = [Tier::Worker, Tier::Normal, Tier::Investigator];
    for (left_index, left) in ordered.iter().enumerate() {
        for (right_index, right) in ordered.iter().enumerate() {
            assert_eq!(left.cmp(right), left_index.cmp(&right_index));
            assert_eq!(left.partial_cmp(right), Some(left_index.cmp(&right_index)));
        }
    }
}
