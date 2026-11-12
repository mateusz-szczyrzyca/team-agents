```
# fn main() -> Result<(), agent_core::state::InvalidTimestamp> {
let at: agent_core::state::Timestamp = "2026-11-11T20:15:54+01:00".parse()?;
assert_eq!(at.to_string(), "2026-11-11T19:15:54Z");
# Ok(())
# }
```

The only way to a value is through the checks:

```compile_fail,E0603
let at = agent_core::state::Timestamp(time::UtcDateTime::UNIX_EPOCH);
```
