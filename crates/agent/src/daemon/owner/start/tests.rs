use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;

use super::*;

#[test]
fn a_new_session_id_is_a_version_4_uuid() {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(7);

    for _ in 0..200 {
        let id = new_session_id(&mut rng).unwrap();

        let text = id.as_str();
        assert_eq!(text.len(), 36, "{text}");
        let groups: Vec<&str> = text.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "{text}"
        );
        assert!(
            text.chars()
                .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{text}"
        );
        assert_eq!(groups[2].chars().next(), Some('4'), "the version: {text}");
        assert!(
            matches!(groups[3].chars().next(), Some('8' | '9' | 'a' | 'b')),
            "the variant: {text}"
        );
    }
}

#[test]
fn two_session_ids_differ() {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(1);

    let first = new_session_id(&mut rng).unwrap();
    let second = new_session_id(&mut rng).unwrap();

    assert_ne!(first, second);
}
