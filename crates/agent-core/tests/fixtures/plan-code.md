Make the validator refuse empty names, and say which line of a settings file is wrong.

1. Parse
[x] 1.1 Read the settings into a typed value
    Files: crates/settings/src/parse.rs
    The parser returns a value whose fields cannot be invalid.
    Check: cargo test -p settings parse
[>] 1.2 Report the line of a bad value
    Files: crates/settings/src/parse.rs, crates/settings/src/parse/tests.rs
    The error carries the line, not only the value.
    Check: reviewer
2. Publish
[ ] 2.1 Raise the crate's version and say what changed
    Files: Cargo.toml, Cargo.lock, WhatsNew.md
    Check: cargo test -p settings --locked
[ ] 2.2 Run the gate on the final commit
    Check: make gate
