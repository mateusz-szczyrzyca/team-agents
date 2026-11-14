//! The longest text of one field the history keeps: the limit's own rules, and the cut of a longer text at a
//! character boundary. The property is checked over texts drawn from a seeded source: a failure names its seed.

#[cfg(test)]
mod history_text {

    use agent_core::history::{
        Cut, DEFAULT_MAX_TEXT_BYTES, InvalidMaxTextBytes, LARGEST_MAX_TEXT_BYTES, MaxTextBytes, cut_text,
    };
    use rand::rngs::Xoshiro256PlusPlus;
    use rand::{Rng, RngExt, SeedableRng};

    /// The limit of `bytes`, which the test knows is valid.
    fn limit(bytes: u32) -> MaxTextBytes {
        MaxTextBytes::new(bytes).unwrap()
    }

    #[test]
    fn a_limit_is_one_byte_to_the_largest_and_the_default_is_a_mebibyte() {
        assert_eq!(MaxTextBytes::new(1).unwrap().get(), 1);
        assert_eq!(
            MaxTextBytes::new(LARGEST_MAX_TEXT_BYTES).unwrap().get(),
            LARGEST_MAX_TEXT_BYTES
        );
        assert_eq!(MaxTextBytes::default().get(), DEFAULT_MAX_TEXT_BYTES);
        assert_eq!(DEFAULT_MAX_TEXT_BYTES, 1_048_576);
    }

    #[test]
    fn zero_and_more_than_the_largest_are_refused_with_the_number() {
        assert_eq!(MaxTextBytes::new(0), Err(InvalidMaxTextBytes { value: 0 }));
        assert_eq!(
            MaxTextBytes::new(LARGEST_MAX_TEXT_BYTES + 1),
            Err(InvalidMaxTextBytes {
                value: LARGEST_MAX_TEXT_BYTES + 1
            })
        );
        assert_eq!(
            MaxTextBytes::try_from(u32::MAX),
            Err(InvalidMaxTextBytes { value: u32::MAX })
        );
    }

    #[test]
    fn a_limit_is_read_from_a_number_and_a_bad_number_is_refused_when_read() {
        assert_eq!(serde_json::from_str::<MaxTextBytes>("4096").unwrap(), limit(4096));
        assert_eq!(serde_json::to_string(&limit(4096)).unwrap(), "4096");
        assert!(serde_json::from_str::<MaxTextBytes>("0").unwrap_err().is_data());
        assert!(serde_json::from_str::<MaxTextBytes>("-1").unwrap_err().is_data());
    }

    #[test]
    fn a_text_of_exactly_the_limit_one_under_and_one_over() {
        let under = cut_text("abcd", limit(5));
        let exactly = cut_text("abcde", limit(5));
        let over = cut_text("abcdef", limit(5));

        assert_eq!((under.text.as_str(), under.was_cut), ("abcd", false));
        assert_eq!((exactly.text.as_str(), exactly.was_cut), ("abcde", false));
        assert_eq!((over.text.as_str(), over.was_cut), ("abcde", true));
    }

    #[test]
    fn an_empty_text_is_kept_whole() {
        let kept = cut_text("", limit(1));

        assert_eq!((kept.text.as_str(), kept.was_cut), ("", false));
    }

    #[test]
    fn a_text_is_cut_before_the_character_that_does_not_fit() {
        // Two bytes: `é`; three: `€`; four: `😀`.
        for (text, max, expected) in [
            ("ééé", 5, "éé"),
            ("ééé", 6, "ééé"),
            ("€€€€", 7, "€€"),
            ("€€€€", 8, "€€"),
            ("😀😀😀", 7, "😀"),
            ("😀😀😀", 8, "😀😀"),
            ("😀😀😀", 3, ""),
            ("a😀", 4, "a"),
            ("a😀", 5, "a😀"),
        ] {
            let kept = cut_text(text, limit(max));

            assert_eq!(kept.text, expected, "{text:?} at {max} bytes");
            assert_eq!(kept.was_cut, expected != text, "{text:?} at {max} bytes");
        }
    }

    /// A character of one, two, three or four bytes, drawn.
    fn character(rng: &mut impl Rng) -> char {
        match rng.random_range(0..4_u8) {
            0 => char::from(rng.random_range(b'a'..=b'z')),
            1 => 'é',
            2 => '€',
            _ => '😀',
        }
    }

    #[test]
    fn what_is_kept_is_the_longest_prefix_that_fits_for_any_text() {
        for seed in 0..500_u64 {
            let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
            let length = rng.random_range(0..30_usize);
            let text: String = (0..length).map(|_| character(&mut rng)).collect();
            let max = rng.random_range(1..=40_u32);

            let kept = cut_text(&text, limit(max));

            let fits = u32::try_from(text.len()).unwrap() <= max;
            assert_eq!(kept.was_cut, !fits, "seed {seed}: {text:?} at {max}");
            assert!(
                text.starts_with(&kept.text),
                "seed {seed}: {kept:?} is not a prefix of {text:?}"
            );
            assert!(
                kept.text.len() <= usize::try_from(max).unwrap(),
                "seed {seed}: {kept:?} over {max}"
            );
            let rest = text.strip_prefix(kept.text.as_str()).unwrap();
            if let Some(next) = rest.chars().next() {
                assert!(
                    kept.text.len() + next.len_utf8() > usize::try_from(max).unwrap(),
                    "seed {seed}: {next:?} would have fit after {kept:?} at {max}"
                );
            }
        }
    }

    #[test]
    fn a_cut_set_names_each_field_that_was_cut_once() {
        let mut cut = Cut::default();

        let prompt = cut.keep("prompt", "abcdef", limit(3));
        let message = cut.keep("final_message", "ab", limit(3));
        let again = cut.keep("prompt", "uvwxyz", limit(3));

        assert_eq!(
            (prompt.as_str(), message.as_str(), again.as_str()),
            ("abc", "ab", "uvw")
        );
        assert!(cut.has("prompt"));
        assert!(!cut.has("final_message"));
        assert_eq!(cut.fields().collect::<Vec<_>>(), ["prompt"]);
        assert!(!cut.is_empty());
        assert!(Cut::default().is_empty());
    }
}
