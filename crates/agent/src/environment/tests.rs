use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use agent_core::config::ValueRule;

use super::*;

#[test]
fn an_absolute_path_is_the_home() {
    let environment = Environment::new(Some(OsString::from("/srv/agents-home")));

    let home = environment.home().unwrap();

    assert_eq!(home.root(), Path::new("/srv/agents-home"));
}

#[test]
fn an_unset_variable_says_what_to_set() {
    let error = Environment::new(None).home().unwrap_err();

    assert!(matches!(error, EnvironmentError::NoHome), "{error:?}");
    assert!(error.to_string().contains("AGENTS_HOME"), "{error}");
}

#[test]
fn a_relative_path_is_refused() {
    for value in ["agents", "./agents", "~/agents", ""] {
        let error = Environment::new(Some(OsString::from(value))).home().unwrap_err();

        assert!(
            matches!(&error, EnvironmentError::NotAbsolute { value: v, .. } if v == value),
            "{value}: {error:?}"
        );
    }
}

#[test]
fn a_path_with_a_nul_is_refused() {
    let error = Environment::new(Some(OsString::from("/srv/a\0b"))).home().unwrap_err();

    assert!(
        matches!(&error, EnvironmentError::NotAbsolute { source, .. } if source.rule == ValueRule::ForbiddenChar('\0')),
        "{error:?}"
    );
}

#[test]
fn a_path_that_is_not_text_is_refused() {
    let value = OsString::from_vec(vec![b'/', 0xff, 0xfe]);

    let error = Environment::new(Some(value.clone())).home().unwrap_err();

    assert!(
        matches!(&error, EnvironmentError::NotText { value: v } if *v == value),
        "{error:?}"
    );
}

#[test]
fn an_environment_made_by_hand_is_in_utc_and_has_no_complaint() {
    let environment = Environment::new(None);

    assert_eq!(environment.local_offset(), time::UtcOffset::UTC);
    assert!(environment.offset_error().is_none());
}
