//! Reading finding numbers from integers. JSON object keys are always text, so these tests give the deserializers
//! the integers themselves, as a deserializer other than `serde_json` does.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::de::IntoDeserializer;
use serde::de::value::{Error as ValueError, MapDeserializer, SeqDeserializer};

use super::{deserialize_map, deserialize_set};

/// Reads the map with the single key `key` and the value `"v"` through [`deserialize_map`].
fn one_key_map<K>(key: K) -> Result<BTreeMap<NonZeroU32, String>, ValueError>
where
    K: IntoDeserializer<'static, ValueError>,
{
    deserialize_map(MapDeserializer::new(std::iter::once((key, "v"))))
}

/// The finding numbers of a map read by [`deserialize_map`], in order.
fn keys_of(map: BTreeMap<NonZeroU32, String>) -> Vec<u32> {
    map.keys().map(|key| key.get()).collect()
}

/// Asserts that reading a map failed and that the failure names the number `number` as a finding number.
fn assert_refused(result: Result<BTreeMap<NonZeroU32, String>, ValueError>, number: &str) {
    let Err(err) = result else {
        panic!("{number} is not a finding number and must be refused");
    };
    let message = err.to_string();
    assert!(
        message.contains(&format!("invalid finding number {number}")),
        "error should name the key {number}: {message}"
    );
}

#[test]
fn integer_keys_of_every_width_at_their_largest_are_finding_numbers() {
    assert_eq!(keys_of(one_key_map(u8::MAX).unwrap()), vec![u32::from(u8::MAX)]);
    assert_eq!(keys_of(one_key_map(u16::MAX).unwrap()), vec![u32::from(u16::MAX)]);
    assert_eq!(keys_of(one_key_map(u32::MAX).unwrap()), vec![u32::MAX]);
    assert_eq!(keys_of(one_key_map(u64::from(u32::MAX)).unwrap()), vec![u32::MAX]);
    assert_eq!(keys_of(one_key_map(i8::MAX).unwrap()), vec![127]);
    assert_eq!(keys_of(one_key_map(i16::MAX).unwrap()), vec![32_767]);
    assert_eq!(keys_of(one_key_map(i32::MAX).unwrap()), vec![i32::MAX.unsigned_abs()]);
    assert_eq!(
        keys_of(one_key_map(i64::from(i32::MAX)).unwrap()),
        vec![i32::MAX.unsigned_abs()]
    );
}

#[test]
fn integer_keys_that_are_not_positive_finding_numbers_are_refused_with_the_number() {
    assert_refused(one_key_map(0u32), "0");
    assert_refused(one_key_map(0u64), "0");
    assert_refused(one_key_map(-1i8), "-1");
    assert_refused(one_key_map(-1i64), "-1");
    assert_refused(one_key_map(i64::from(i32::MIN)), &i32::MIN.to_string());
    assert_refused(one_key_map(u64::from(u32::MAX) + 1), "4294967296");
}

#[test]
fn integer_elements_of_a_set_are_finding_numbers() {
    let set = deserialize_set(SeqDeserializer::<_, ValueError>::new(vec![3u32, 1].into_iter())).unwrap();
    assert_eq!(set.iter().map(|key| key.get()).collect::<Vec<_>>(), vec![1, 3]);
}
