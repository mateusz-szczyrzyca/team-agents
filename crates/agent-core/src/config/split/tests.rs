use super::Split;
use crate::config::Config;

#[test]
fn split_configuration_defaults_overrides_and_template_agree() {
    for (text, expected) in [
        (
            "",
            Split {
                part_limit: 1500,
                corrections: 2,
            },
        ),
        (
            "[split]",
            Split {
                part_limit: 1500,
                corrections: 2,
            },
        ),
        (
            "[split]\npart_limit=0",
            Split {
                part_limit: 0,
                corrections: 2,
            },
        ),
        (
            "[split]\ncorrections=0",
            Split {
                part_limit: 1500,
                corrections: 0,
            },
        ),
        (
            "[split]\npart_limit=40\ncorrections=5",
            Split {
                part_limit: 40,
                corrections: 5,
            },
        ),
    ] {
        assert_eq!(Config::from_toml_str(text).unwrap().split(), expected, "{text}");
    }
    let template = include_str!("../../../../../config/agents.example.toml");
    assert!(template.contains("[split]"));
    assert_eq!(Config::from_toml_str(template).unwrap().split(), Split::default());
}

#[test]
fn split_configuration_rejects_unknown_negative_fractional_and_overflowing_values() {
    for field in ["part_limit", "corrections"] {
        for value in ["-1", "1.5", "4294967296", "\"2\""] {
            assert!(matches!(
                Config::from_toml_str(&format!("[split]\n{field}={value}")),
                Err(crate::config::ConfigError::Parse(_))
            ));
        }
    }
    assert!(matches!(
        Config::from_toml_str("[split]\nunknown=1"),
        Err(crate::config::ConfigError::Parse(_))
    ));
}

#[test]
fn split_estimate_unit_defaults_and_accepts_non_code_units_but_rejects_blank_text() {
    for text in ["", "[split]", include_str!("../../../../../config/agents.example.toml")] {
        assert_eq!(Config::from_toml_str(text).unwrap().split_unit(), "lines");
    }
    for unit in ["pages", "images", "страницы"] {
        let config = Config::from_toml_str(&format!("[split]\nunit={unit:?}"));
        assert!(
            matches!(config, Ok(ref parsed) if parsed.split_unit() == unit),
            "{config:?}"
        );
    }
    for text in ["[split]\nunit=''", "[split]\nunit='  '", "[split]\nunit=42"] {
        assert!(matches!(
            Config::from_toml_str(text),
            Err(crate::config::ConfigError::Parse(_))
        ));
    }
}
