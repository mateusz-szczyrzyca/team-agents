use std::os::unix::ffi::OsStringExt;

use super::*;

#[test]
fn cuts_all_configured_values_and_suffix_selected_environment_values_exactly() {
    let secrets = Secrets::parse(
        "github_token = 'github-value'\n[agent_env]\nMODEL = 'modèle-value'\n[notification_env]\nCHAT = 'chat-value'\n",
        0o600,
    )
    .unwrap();
    let environment = [
        ("AUTH_TOKEN", "token-value"),
        ("MY_SECRET", "secret-value"),
        ("DB_PASSWORD", "password-value"),
        ("API_KEY", "key-value"),
        ("NOT_TOKEN_SUFFIX", "public-value"),
        ("lowercase_key", "case-sensitive-value"),
        ("EMPTY_KEY", ""),
    ]
    .map(|(key, value)| (OsString::from(key), OsString::from(value)));
    let cut = SecretsCut::new(&secrets, environment);

    assert_eq!(
        cut.redact("github-value modèle-value chat-value token-value secret-value password-value key-value public-value case-sensitive-value github-ValuE".as_bytes()),
        b"<cut> <cut> <cut> <cut> <cut> <cut> <cut> public-value case-sensitive-value github-ValuE"
    );
}

#[test]
fn cuts_longest_overlapping_values_and_repeated_occurrences_without_redacting_debug() {
    let secrets = Secrets::parse("github_token = 'abc'\n", 0o600).unwrap();
    let cut = SecretsCut::new(&secrets, [("LONG_SECRET".into(), "abcdef".into())]);

    assert_eq!(cut.redact(b"abcdef abc abcdef"), b"<cut> <cut> <cut>");
    assert!(!format!("{cut:?}").contains("abc"));
}

#[test]
fn preserves_binary_bytes_and_cuts_non_utf8_environment_values() {
    let cut = SecretsCut::new(
        &Secrets::default(),
        [("BINARY_KEY".into(), OsString::from_vec(vec![0xfe, 0xfd]))],
    );

    assert_eq!(cut.redact(&[0xff, 0xfe, 0xfd, 0x00]), b"\xff<cut>\x00");
    assert_eq!(cut.redact(&[]), Vec::<u8>::new());
}
