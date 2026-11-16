//! Tests the greeting's exact format and preservation of names.

use super::greet;

#[test]
fn greets_names_without_changing_them() {
    for (name, expected) in [
        ("Ada", "Hello, Ada!"),
        ("Grace", "Hello, Grace!"),
        ("", "Hello, !"),
        ("Łukasz 🌍", "Hello, Łukasz 🌍!"),
        (" Ada ", "Hello,  Ada !"),
    ] {
        assert_eq!(greet(name), expected);
    }
}
