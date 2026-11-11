# Rust developer rules

You write Rust code. A reviewer checks every pull request against the same rule IDs (`rust-review.md`).

**Every rule is binding.** If a rule cannot or clearly should not apply at one specific spot, deviate as little as possible and record the deviation and its reason under *Deviations* in the PR description. The reviewer decides. An unrecorded deviation is a defect.

**Precedence.** The task and the repository's own documents (`AGENTS.md`, `CONTRIBUTING.md`, CI configuration) decide project specifics — commands, layout, toolchain, MSRV, naming, commit style — and may add rules. They relax a rule in this file only by saying so explicitly. Existing code that breaks these rules is not a precedent: do not copy it, do not fix it in passing, list it under *Out of scope*.

## 1. Non-negotiables

1. **Test first.** No production code without a failing test that demanded it. (§5)
2. **No error is ever ignored.** Not one. (§7)
3. **No panic paths in production code:** no `unwrap`, `expect`, `panic!`, `todo!`; no unchecked indexing, casts or arithmetic. (§6, §7)
4. **Types before checks.** Whatever can be a compile error must not be a runtime check. (§6)
5. **`thiserror` for errors;** `anyhow` only at the top of a binary. (§7)
6. **Green gate, run by you, on the final commit:** rustfmt, clippy with `-D warnings`, tests, docs. (§4)
7. **Never weaken the gate:** no deleted, ignored or loosened tests, no `#[allow]`, no lint or CI configuration edits to get green. (§4, §5)
8. **At most 1500 changed lines per PR, tests not counted.** A bigger change becomes several PRs. (§16)
9. **Stay in scope.** Do what the task needs and nothing else. (§16)
10. **Report only what you ran and saw.** (§3)

## 2. Workflow

1. Read the task, `AGENTS.md`, `Cargo.toml` (edition, `rust-version`, features, `[lints]`), `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`, the CI configuration, and the code and tests around the change.
2. Requirement unclear or contradictory → ask. If you cannot ask, take the most conservative reading and state the assumption in the PR.
3. Estimate the size of the change outside tests — tests do not count toward the limit. Near or over it → plan the PR stack before writing code (§16).
4. Design the types and error enums first (§6, §7), then build the behaviour test-first (§5).
5. Run the gate (§4). Fix causes, not symptoms.
6. Review your own `git diff <base>...HEAD` the way the reviewer will. Search the added lines for `unwrap(`, `expect(`, `let _ =`, `.ok()`, `unwrap_or`, `if let Ok(`, `Err(_)`, `map_err(|_|`, `filter_map`, `#[allow`, `#[ignore`, ` as `, `unsafe`, `TODO`, `dbg!`, `println!` — every hit must be justified by these rules. Re-measure the size.
7. Write the PR description (§17).

## 3. Working discipline — `GEN`

- **GEN-1 Look before you write.** Reuse the repository's existing types, helpers and patterns. Do not duplicate what is already there.
- **GEN-2 Verify every API you call.** Check the docs or source of the exact version in `Cargo.lock` instead of recalling it. Compile early and often: the compiler is the authority.
- **GEN-3 No placeholders.** No stub bodies, no "implement later", no hard-coded return values, no fake data paths. What you commit works, or is not there.
- **GEN-4 Never implement to the test.** No special-casing of the inputs the tests use. The code is correct for every input its types admit.
- **GEN-5 Do not hand-edit generated files, vendored code or lock files.** Regenerate them with the tool that owns them.
- **GEN-6 No narration comments** ("changed X to Y", "new", "fixed"). That belongs in the commit message.
- **GEN-7 Report only what you ran and saw.** Quote real command results. If something could not be run, or you are unsure or blocked, say so. Never present a guess as a result.

## 4. Gate, formatting, lints — `LINT`

Use the repository's gate command if it defines one (`AGENTS.md`, Makefile, justfile, CI). Otherwise:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features          # runs doctests too
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

Add `--locked` when `Cargo.lock` is committed. Run whatever else the repository's CI runs (`cargo deny`, `cargo audit`, `cargo nextest`, `cargo hack`, `cargo semver-checks`, Miri, …).

- **LINT-1** Format with `cargo fmt --all` before every commit. Never hand-format against rustfmt. `#[rustfmt::skip]` only on hand-aligned tables, with a comment.
- **LINT-2** Zero warnings from rustc, clippy and rustdoc. Fix the code, not the message.
- **LINT-3 Suppressing a lint is a last resort,** and only as `#[expect(lint, reason = "…")]` on the smallest item that needs it — it fails the gate again once the lint stops firing, so it cannot go stale. Never `#[allow]`. Never crate- or module-wide. Never `warnings`, `clippy::all` or `clippy::pedantic` wholesale. A lint that fires only under some `cfg` gets a conditional expectation: `#[cfg_attr(<cfg>, expect(…))]`. Every suppression is listed in the PR.
- **LINT-4** Never edit lint, format, toolchain or CI configuration to get green (`[lints]`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`, `deny.toml`, workflows). A configuration change is its own PR, and only when the task asks for it.
- **LINT-5** No `#![deny(warnings)]` in source: it breaks the build on every toolchain update. Strictness lives in the gate.
- **LINT-6** Stay inside the repository's edition and MSRV (`rust-version`). Stable features only, unless the repository is on nightly.
- **LINT-7** In a repository without a strict lint setup, follow these rules by hand, check your own change by passing the appendix lints as `-W clippy::<lint>` flags, and propose the baseline from the appendix as a separate PR.

## 5. Test-driven development — `TDD`

- **TDD-1 Red → green → refactor, one behaviour at a time.**
  1. *Red.* Write one small test for the next behaviour. Run it. Watch it fail **for the right reason**. A compile error is not red: add the thinnest stub (`todo!()` is allowed here, and only here — it is never committed) and see the test itself fail.
  2. *Green.* Write the least code that passes. Nothing the tests do not demand.
  3. *Refactor.* Clean up with the tests green. Re-run after every step.
- **TDD-2 No behaviour without a test.** Every new function, branch, error variant and bug fix has a test that fails when that behaviour is removed. Check it: break or revert the new code and see the new tests go red.
- **TDD-3** A bug fix starts with a failing regression test that reproduces the bug.
- **TDD-4** Refactoring changes neither behaviour nor tests. Code without coverage gets characterisation tests first.
- **TDD-5** Test behaviour through the public interface, not implementation details. A refactoring must not break tests.
- **TDD-6** Cover the happy path, every error variant (triggered through the public API — inject a failing dependency if that is what it takes), the boundaries (empty, zero, one, max, off-by-one, overflow), invalid input, and non-ASCII text wherever text is handled.
- **TDD-7 Assert precisely.** `assert_eq!` on whole values. Errors by variant and fields (`matches!`, `let … else`), never by comparing `Display` or `Debug` strings, never a bare `assert!(r.is_err())` or `assert!(r.is_ok())`.
- **TDD-8 Deterministic and isolated.** No real network, wall clock, `sleep`, unseeded randomness, fixed ports, shared global state, environment mutation, dependence on test order, or files outside a `tempfile` directory. Inject the clock, the RNG and the I/O. Async time runs on the runtime's paused clock, not on real sleeps.
- **TDD-9 Readable.** The name states behaviour and condition (`returns_not_found_when_user_is_missing`). Arrange, act, assert. One reason to fail. Table-driven cases instead of copy-paste.
- **TDD-10 The right kind of test.** Unit tests in `#[cfg(test)] mod tests` beside the code. Integration tests in `tests/`, through the public API only. Doctests for public examples. Property tests for parsers, codecs (round-trip) and invariants. Compile-fail tests (`trybuild`, or a `compile_fail` doctest) for guarantees a type claims to give. Miri for `unsafe`. `loom` for hand-written synchronisation. Use the tools the repository already has.
- **TDD-11 Existing tests are a contract.** Never delete, `#[ignore]`, comment out or loosen a test to get green. Change an existing test only when the task explicitly changes the behaviour it specifies, and list every modified or removed test with its reason in the PR. No skipped, empty, placeholder or assertion-free tests.
- **TDD-12** No test-only paths in production code: no `if cfg!(test)`, no flags or wider visibility just for tests. Test helpers live under `#[cfg(test)]` or in `tests/`.
- **TDD-13** Prefer real implementations and in-memory fakes. Mock only process boundaries (network, clock, OS), behind a trait you own.

Exempt from test-first: documentation, comments, formatting, pure moves and renames, dependency or configuration changes that alter no behaviour.

In test code `unwrap`, `expect`, `panic!` and indexing are fine — a panic is a failed test. Shared helpers in `tests/` return `Result`. An example test is in §7.

## 6. Types: make invalid states unrepresentable — `TYP`

For every invariant, take the highest rung you can reach:

1. **Unrepresentable** — the wrong program does not compile.
2. **Checked once, at the boundary** — a constructor returns `Result`; from then on the type carries the proof.
3. **Checked at every use** — returns `Result` each time.

Never: unchecked, assumed, or merely documented.

- **TYP-1 Parse, don't validate.** Turn raw input into a domain type at the boundary (`TryFrom`, `FromStr`, `new` → `Result`). Code behind the boundary takes the domain type and never checks again.
- **TYP-2 Seal the invariant.** Private fields, and no way around the constructor. That includes derives: no `Default` that yields an invalid value, `Deserialize` only through validation (`#[serde(try_from = "…")]`), no `pub` field, setter or `DerefMut` that can break it.
- **TYP-3 Newtypes for domain values.** Ids, quantities with units, validated strings (`UserId`, `Millimetres`, `Email`) — not bare `u64`, `f64`, `String`. Two arguments of the same primitive type must not be swappable by accident. Use the strong types std already has: `Duration`, `Instant`, `SystemTime`, `Path`/`PathBuf`, `IpAddr`, `SocketAddr`, `NonZero*`.
- **TYP-4 Enums, not flags, strings or magic values.** No `bool` parameters: use a two-variant enum. No state, kind or mode as a string or integer code. No sentinel values (`-1`, `0`, `""` meaning "none"): use `Option` or an enum.
- **TYP-5 One state, one variant.** A struct with correlated `Option` or `bool` fields becomes an enum whose variants carry exactly the data that is valid in that state.
- **TYP-6 Exhaustive `match`.** No `_ =>` arm on an enum defined in this workspace: adding a variant must break the build everywhere a decision is needed. Wildcards are for foreign `#[non_exhaustive]` enums and open domains (integers, strings).
- **TYP-7 Encode protocols in ownership.** One-shot operations take `self` by value. A call order that matters becomes typestate (`Connection<Handshaking>` → `Connection<Ready>`). Paired actions (lock/unlock, begin/commit) are RAII guards, not "remember to call X".
- **TYP-8 Checked numerics.** Integer conversions with `From` / `TryFrom`, never `as` — it truncates and wraps silently. Arithmetic on values not proven small uses `checked_*`, `saturating_*` or `wrapping_*`, chosen on purpose: overflow wraps silently in release builds. No floats for money or other exact quantities.
- **TYP-9 No unchecked access.** `get`, `first`, `last`, iterators, slice patterns, `split_first`, `chunks_exact` — not `v[i]` or `&s[a..b]`, unless the type proves the bounds (`[T; N]` with a constant index). No division or remainder by a value not proven non-zero.
- **TYP-10 Compile time beats run time.** Generics and `impl Trait` over `dyn Any` and downcasts. Structs over `HashMap<String, Value>` bags. The borrow checker over `RefCell`. `const _: () = assert!(…);` for relationships between constants. `#[must_use]` on types and functions whose result must not be dropped. A panic in a `const` initialiser is a compile error, so `const N: NonZeroU32 = NonZeroU32::new(8).unwrap();` is fine.
- **TYP-11 Stay proportionate.** When the type machinery would cost more clarity than the bug it prevents, step down one rung — to a validated newtype, never to "unchecked".

```rust
// BAD: meaningless values compile — running with an exit code, port 0, mode "raed".
struct Job {
    running: bool,
    exit_code: Option<i32>,
    port: u16,
    mode: String,
}

// GOOD: every value of these types is a legal state. (Doc comments left out of this sketch.)
enum Job {
    Queued,
    Running { port: Port, mode: Mode },
    Finished { exit: ExitStatus },
}

enum Mode {
    Read,
    Write,
}

/// TCP port; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(try_from = "u16")] // deserialization cannot bypass the check
pub struct Port(NonZeroU16);

impl TryFrom<u16> for Port {
    type Error = PortError;

    fn try_from(raw: u16) -> Result<Self, Self::Error> {
        NonZeroU16::new(raw).map(Self).ok_or(PortError::Zero)
    }
}
```

## 7. Errors — `ERR`

- **ERR-1 Nothing is ignored.** Every fallible outcome is (a) propagated with `?`, with context; or (b) recovered by a fallback that is specified behaviour and has a test; or (c) — only where it cannot be propagated: in `Drop`, or during cleanup while another error is already being returned — logged at `warn` or `error` with context, next to a comment saying why. "Cannot happen" and "does not matter" are not handling. If carrying on really is right, write the decision down: match the error, log it, comment why.
- **ERR-2 All of these count as ignoring:**
  - `let _ = f();`, `_ = f();`, `f().ok();`, an empty or catch-all `Err(_) => {}` arm;
  - `if let Ok(x) = f()` or `while let Ok(x) = …` with the `Err` case unhandled;
  - `.ok()`, `.unwrap_or(…)`, `.unwrap_or_default()`, `.unwrap_or_else(|_| …)` used to make a failure disappear rather than as a specified fallback;
  - `filter_map(Result::ok)`, `.flatten()` or `.flat_map(…)` over `Result`s — failed items vanish; collect into `Result<Vec<_>, _>` or handle each error;
  - `map_err(|_| …)` — the cause is thrown away;
  - concurrency: a dropped `JoinHandle` (thread or task), an unchecked `JoinError`, an ignored `send` / `recv` error or timeout;
  - I/O: `write` for `write_all`, `read` for `read_exact` (byte count ignored); a `BufWriter` dropped without `flush()?`; data that must survive a crash without `sync_all()?`; `Command::status()` / `output()` without checking `ExitStatus::success()`; a child process never waited for;
  - lossy shortcuts — `as` casts, `from_utf8_lossy`, `to_string_lossy` — unless losing data is the specified behaviour;
  - a `None` that means failure (lookup miss, `checked_*` overflow, `.parse().ok()`): convert it with `ok_or_else` and propagate;
  - `mutex.lock().unwrap()`: map poisoning to an error, or recover on purpose with `PoisonError::into_inner` and a comment why the data is still consistent;
  - unchecked FFI return codes and null pointers;
  - logging the error and continuing as if the operation had succeeded.
- **ERR-3 No panic paths in production code** — library or binary: no `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, and no `assert!` on anything derived from input. A panic means "bug", never "bad input" or "I/O failed". If failure is truly impossible, first make the types say so (§6). Only when they cannot: `expect("<the invariant>")` under a local `#[expect(clippy::expect_used, reason = "…")]`. `debug_assert!` and `assert!` may state internal invariants; document them under `# Panics`.
- **ERR-4 `thiserror` by default.** Every library crate and every module with domain logic returns its own typed error. `anyhow` is allowed only in the top layer of a binary (`main` and the glue that merely reports errors to a human), in tests, examples, benches and build or xtask scripts — never in a library's public API. Never `Box<dyn Error>`, `String`, `&str`, `()`, `bool` or `Option` to signal failure. No other error crates (`eyre`, `snafu`, …) unless the repository already uses them.
- **ERR-5 Narrow error types.** One enum per module or operation, holding only the failures that can really happen there — not one crate-wide `Error`. A caller can match it exhaustively with no dead arms. Compose upwards with `#[source]` or `#[from]`.
- **ERR-6 Variants say what failed, in domain terms, with the context needed to act:** path, id, key, offending value (never a secret). Not `Io(#[from] io::Error)` — which file, which operation? Use `#[from]` only where one source type has exactly one meaning; otherwise build the variant in `map_err(|source| …)`.
- **ERR-7 Keep the cause chain.** Wrap the underlying error as the `source` (`#[source]`, `#[from]`, `#[error(transparent)]`). Never stringify it into a field, never drop it. Do not also print the source in the wrapper's message: reporters walk `source()` and would show it twice.
- **ERR-8 Error type hygiene.** Messages are lowercase, concise, without a trailing full stop, and name the failed operation and value. Types are `Debug` + `Error` + `Send + Sync + 'static`, small (box large payloads), and `#[non_exhaustive]` when public and likely to grow. No third-party error type in a library's public API unless that dependency is deliberately part of the API.
- **ERR-9 With `anyhow`:** `.context(…)` or `.with_context(|| …)` on every `?` that crosses a boundary (what was being done, with which input); print the whole chain (`{:#}` or `{:?}`); never `downcast` to decide what to do — an error you branch on needs a type.
- **ERR-10 Handle each error once.** Return it (with context) or handle and log it — not both.
- **ERR-11 Binaries** return `ExitCode` or `Result` from `main`: non-zero exit on failure, the full cause chain on stderr. No `std::process::exit` outside `main` — it skips destructors. Program output goes through an injected `impl Write`, not `println!`: testable, and a closed pipe becomes an `Err` instead of a panic.
- **ERR-12 Fallible teardown gets a method.** `Drop` cannot return an error and must never panic. A type whose cleanup can fail (flush, commit, close) offers `fn close(self) -> Result<…>`; `Drop` is only a best-effort fallback that logs.
- **ERR-13 Document failure.** Every `pub fn` that returns `Result` has `# Errors` (which variants, when). Anything that can panic has `# Panics`.

```rust
/// Why [`load_settings`] failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoadSettingsError {
    /// The settings file could not be read.
    #[error("cannot read settings file {path:?}")]
    Read {
        /// File that was being read.
        path: PathBuf,
        /// OS failure: kept as the cause, not repeated in the message.
        source: std::io::Error,
    },
    /// The file is not valid settings JSON.
    #[error("invalid settings in {path:?}")]
    Parse {
        /// File that was being parsed.
        path: PathBuf,
        /// Parser's reason, with line and column.
        source: serde_json::Error,
    },
}

/// Loads the settings stored at `path`.
///
/// # Errors
/// [`LoadSettingsError::Read`] if the file cannot be read,
/// [`LoadSettingsError::Parse`] if it is not valid settings JSON.
pub(crate) fn load_settings(path: &Path) -> Result<Settings, LoadSettingsError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadSettingsError::Read {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| LoadSettingsError::Parse {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_settings_reports_the_path_when_the_file_is_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("settings.json");

        let err = load_settings(&missing).expect_err("the file does not exist");

        assert!(matches!(&err, LoadSettingsError::Read { path, .. } if *path == missing));
    }
}
```

Common replacements:

```rust
// Fail on the first bad item.           BAD: .filter_map(|p| p.parse().ok()) — bad items vanish
let nums = parts.iter().map(|p| p.parse::<u32>()).collect::<Result<Vec<_>, _>>().map_err(Error::BadItem)?;

// Keep the cause.                        BAD: .map_err(|_| Error::BadId)
let id = raw.parse::<u64>().map_err(|source| Error::BadId { raw: raw.to_owned(), source })?;

// `Ok(status)` only means the process ran.   BAD: Command::new("git").arg("fetch").status()?;
let status = Command::new("git").arg("fetch").status().map_err(Error::Spawn)?;
if !status.success() {
    return Err(Error::GitFetch { status });
}

// Whole buffer, flushed before the writer is dropped.   BAD: out.write(buf)?;
out.write_all(buf).map_err(Error::Write)?;
out.flush().map_err(Error::Write)?;

// A conversion that can fail says so.    BAD: buf.len() as u16
let len = u16::try_from(buf.len()).map_err(|source| Error::TooLong { len: buf.len(), source })?;

// Carrying on is a decision: matched, logged, explained.   BAD: let _ = tx.send(event);
if let Err(err) = tx.send(event) {
    // The receiver closes only during shutdown: nobody is left to notify.
    tracing::debug!(%err, "event dropped: receiver closed");
    return Ok(());
}

// The one sanctioned `expect`: an invariant the types cannot express, justified and scoped.
#[expect(clippy::expect_used, reason = "the pattern is a literal; the `date_re_compiles` test covers it")]
static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("literal regex is valid"));
```

## 8. Design and idioms — `API`

- **API-1 Borrow in, own out.** Parameters are `&str`, `&[T]`, `&Path` — not `&String`, `&Vec<T>`, `&PathBuf`. Take ownership (`String`, `impl Into<String>`) only when the function stores or consumes the value.
- **API-2 No `clone()` to silence the borrow checker.** Restructure: narrower scopes, split borrows, `mem::take`, passing references. Clone when a second owner is really needed. Write `Arc::clone(&x)` for ref-counted handles so cheap clones are visible. `Cow` for values that are usually borrowed.
- **API-3 Minimal visibility.** Private by default, `pub(crate)` for internal sharing, `pub` only for the intended API — every `pub` item is a compatibility promise. Fields are private unless the struct is plain data. Public enums and structs that will grow are `#[non_exhaustive]`.
- **API-4 Standard traits.** `Debug` on every type (hand-written and redacting where it holds secrets). `Clone`, `PartialEq`/`Eq`, `Hash`, `Ord`, `Default` where they are meaningful and preserve the invariants. Conversions through `From`, `TryFrom`, `AsRef`, `FromStr`, `Display`: implement `From`, never `Into`; `From` is infallible and lossless, otherwise it is `TryFrom`. No `Deref` to fake inheritance.
- **API-5 Naming follows the Rust API Guidelines.** `as_` (free borrow), `to_` (costly), `into_` (consuming). Getters without `get_`. `iter` / `iter_mut` / `into_iter`. `is_` / `has_` for `bool`. `try_` for fallible twins. `new` / `with_…` / `from_…` constructors. Acronyms as words (`HttpClient`). Names come from the domain: no `utils`, `helpers`, `common`, `misc` modules, no `Manager`, `Data`, `Info` filler.
- **API-6 Small and flat.** A function does one thing. Early returns (`?`, `let … else`) instead of nesting. As a guide: at most 50 lines and 5 parameters; beyond that, group parameters into a struct or use a builder.
- **API-7 Static before dynamic, concrete before abstract.** Concrete type, then generic or `impl Trait`, then `dyn Trait` only for real runtime heterogeneity. No trait, type parameter, feature flag or configuration option without a second real use.
- **API-8 Iterators and pattern matching** over index loops and `is_some()` + `unwrap()`. A plain `for` loop still beats a chain nobody can read.
- **API-9 No global mutable state.** No `static mut`. `LazyLock` / `OnceLock` only for true constants (a compiled regex, a lookup table). Configuration, clients, clocks and RNGs are passed in — which is also what makes the code testable.
- **API-10 No magic values.** A named `const` with its unit and reason. No repeated literals.
- **API-11 Current idioms, as far as the MSRV allows:** `let … else`, inline format arguments (`{x}`), `is_some_and`, `LazyLock`, native `async fn` in traits, `#[expect]`. Prefer std over the helper crates it has replaced (`lazy_static`, `once_cell`, `async-trait`) unless the repository's convention says otherwise.
- **API-12 Clean modules.** Organised by domain, not by kind; one responsibility each. No glob imports, except `use super::*` in test modules and declared preludes. No dead code, commented-out code, `dbg!`, stray prints, or `TODO` / `FIXME` without an issue reference.

## 9. Documentation — `DOC`

- **DOC-1** Every public item has `///` docs: a one-sentence summary line, then `# Errors`, `# Panics`, `# Safety`, `# Examples` where they apply. Every crate and module opens with `//!` stating its purpose.
- **DOC-2** Every `const`, `static`, type, enum variant and struct field — private ones too — has a doc comment saying what it is for: unit, valid range, invariant.
- **DOC-3** Comments explain *why* (the decision, the invariant, the trade-off, a link to the spec or issue), not what the next line does. A comment that no longer matches the code is a bug.
- **DOC-4** Doc examples compile and run as doctests and use `?`, not `unwrap`. Link items with intra-doc links (`` [`Type`] ``). When behaviour or public API changes, the README, CHANGELOG and docs change in the same PR.

## 10. Unsafe — `UNS`

- **UNS-1 Do not write `unsafe`.** New crates start with `#![forbid(unsafe_code)]`. `unsafe` is acceptable only when the task demands it and no safe alternative exists (std, or an audited crate) — and the PR must flag it for human review.
- **UNS-2** Each `unsafe` block is minimal, holds one unsafe operation, and carries a `// SAFETY:` comment showing why every precondition holds. Every `unsafe fn` and `unsafe trait` documents `# Safety`.
- **UNS-3 Sound by construction.** Unsafe code sits behind a safe API that no safe caller can misuse into undefined behaviour — enforced by types and private fields, not by documentation.
- **UNS-4** Tests exercise the unsafe code, and run under Miri where the repository supports it.
- **UNS-5 Never:** `static mut`; `transmute` where a safe conversion exists; `mem::zeroed` or `assume_init` on unproven data; `get_unchecked`, `unwrap_unchecked`, `unreachable_unchecked` without a benchmark proving the need; `unsafe impl Send` / `Sync` without a written proof; unwinding across FFI. In FFI: `#[repr(C)]`, every pointer and return code checked, ownership documented.

## 11. Concurrency and async — `CON`

- **CON-1 Never block the async runtime.** No `std::thread::sleep`, blocking file, network or process calls, long CPU work, or blocking lock and channel waits inside async code. Use the runtime's async APIs or `spawn_blocking`.
- **CON-2 No lock guard or `RefCell` borrow across `.await`.** Critical sections are short: no I/O, callbacks or awaits while holding a lock. `std::sync::Mutex` for short synchronous sections; an async mutex only when the lock must span an `.await` — and question the design first.
- **CON-3 Share by communicating.** Prefer moving ownership through channels, or one task that owns the state, over `Arc<Mutex<…>>`. Where locks are shared, their order is fixed and documented, and a check and the action that depends on it happen under the same lock.
- **CON-4 Structured concurrency.** Every spawned task or thread has an owner that joins it and handles both its panic (`JoinError`) and its `Result`. No fire-and-forget. Long-lived work supports cancellation and graceful shutdown (`JoinSet`, `CancellationToken`, scoped threads).
- **CON-5 Everything is bounded.** Bounded channels and queues. A concurrency limit (`Semaphore`, `buffer_unordered(n)`). A timeout on every external call. Retries capped, with backoff and jitter, and only for idempotent operations.
- **CON-6 Cancellation safety.** Any `.await` may be the last thing a future does. Never leave an invariant broken across an await. In `select!` use cancel-safe operations, or pin the future outside the loop. Say so in the docs when a public async fn is not cancel-safe.
- **CON-7** No `block_on` inside async code. Libraries do not create runtimes. A function is `async` only if it awaits.
- **CON-8** No hand-rolled synchronisation primitives or lock-free structures unless the task requires them; then every atomic `Ordering` is justified in a comment and the code is tested with `loom`.

## 12. Performance — `PERF`

- **PERF-1 Correct and clear first.** Optimise only with a measurement. A PR that claims a speed-up shows the numbers before and after.
- **PERF-2 Do not waste by default.** No needless `clone`, `to_owned`, `to_string` or `collect`. No avoidable allocation in hot loops. `with_capacity` when the size is known (never from untrusted input). The `entry` API instead of a double lookup. Buffered I/O for many small reads or writes. Regexes compiled once. Big values passed by reference.
- **PERF-3 Watch the complexity.** No accidental O(n²): `Vec::contains`, `remove(0)` or nested scans inside loops become `HashSet`, `HashMap`, `VecDeque`. No N+1 queries. Stream instead of loading unbounded data into memory.

## 13. Security — `SEC`

- **SEC-1 All external input is untrusted** — network, files, environment, CLI, database rows, other services. Parse it into types at the boundary (TYP-1) and bound it: length, size, count, nesting depth, time. Never allocate, loop or recurse in proportion to an unvalidated number.
- **SEC-2 Secrets** never appear in source, tests, logs, error messages, `Debug` / `Display` output or panic messages. Hold them in a redacting type, compare them in constant time, load them from the environment or a secret store.
- **SEC-3 No injection.** SQL through bound parameters only. Processes through `Command::new(…).arg(…)`, never a shell string. Paths built from input are confined: `Path::join` with an absolute path *replaces* the base, so reject absolute paths and `..` components.
- **SEC-4 Cryptography** comes from vetted crates — never home-made primitives or protocols. Security-relevant randomness comes from the OS CSPRNG. TLS verification is never disabled.
- **SEC-5 No check-then-act on the filesystem** (`exists()`, then open). Use atomic operations: `OpenOptions::create_new`, or write to a temporary file and rename. Temporary files come from `tempfile`.
- **SEC-6 Deserializing untrusted data:** size limits, strict schemas where it matters (`deny_unknown_fields`), validated types (TYP-2).
- **SEC-7 Errors sent to remote callers** carry no internals (paths, SQL, stack traces): log the detail, return a sanitised error. No personal data in logs.
- **SEC-8** Never call `std::env::set_var` or `remove_var` after startup: not thread-safe, and `unsafe` since edition 2024. Read the environment once into a typed configuration.

## 14. Logging and output — `OBS`

- **OBS-1** Use the logging facade the repository uses (`tracing` or `log`). Libraries never print: no `println!`, `eprintln!`, `dbg!`.
- **OBS-2** Structured fields, not interpolated strings. Levels mean something: `error` — someone must act; `warn` — degraded but handled; `info` — lifecycle; `debug` / `trace` — diagnostics. With `#[instrument]`, skip the arguments by default and add explicit fields.
- **OBS-3** An error is logged once, where it is handled, with its whole cause chain.

## 15. Dependencies and Cargo — `DEP`

- **DEP-1 A new dependency needs a reason in the PR:** what it is for, why std and the existing dependencies are not enough, and that it is maintained, widely used and licence-compatible. Order of preference: std, then a dependency already present, then a small well-known crate. No dependency for a few lines of code.
- **DEP-2** Add it with the minimal feature set. In a workspace, declare the version once in `[workspace.dependencies]`. No `*` versions; no git or path dependencies on outside repositories unless the task requires it. Commit only the `Cargo.lock` changes your change causes — no blanket `cargo update`.
- **DEP-3** Do not change `edition`, `rust-version`, profiles, features, the toolchain or existing dependency versions unless that is the task.
- **DEP-4 Cargo features are additive:** never mutually exclusive, and the crate builds with any combination.
- **DEP-5** In library crates, public API changes follow semver. A breaking change is called out in the PR, and in the CHANGELOG if there is one.

## 16. Pull requests — `PR`

- **PR-1 Hard limit: 1500 changed lines per PR, tests not counted** — added plus deleted lines of everything hand-written outside tests: production code, docs, configuration. Not counted: test code (`tests/` and `benches/` directories, `tests.rs` files, and the `#[cfg(test)] mod … { }` block that ends a source file), lock files, generated code, snapshots, fixtures, vendored files, pure renames and moves. Measure it (`<base>` is the PR's target branch; add the repository's generated and fixture paths to the exclusions):

  ```sh
  git diff -U100000 <base>...HEAD -- . ':!*Cargo.lock' ':!*.snap' \
      ':!tests/*' ':!*/tests/*' ':!*/tests.rs' ':!benches/*' ':!*/benches/*' |
  awk '
    /^diff --git / { hdr = 1; test = 0; armed = 0; next }
    hdr            { if (/^@@/) hdr = 0; next }
                   { s = substr($0, 2) }
    !test && s ~ /^#\[cfg\(test\)\]/ { armed = 1; next }
    !test && armed && s != "" && s !~ /^[ #)]/ {
        if (s ~ /^(pub(\([a-z]+\))? +)?mod +[A-Za-z0-9_]+ *\{/) test = 1
        armed = 0
    }
    !test && !armed && /^[+-]/ { n++ }
    END { print n + 0 }'
  ```

  1500 is a ceiling, not a target: smaller is better. The limit is never a reason to write fewer tests.
- **PR-2 Plan the split before you write code.** If the estimate (tests aside) passes about 1200 lines, define the stack of PRs up front — do not write 3000 lines and cut them up afterwards. A marginal overshoot (up to about 10 %) is acceptable only when a smaller PR could not build or pass on its own; record it under *Deviations*.
- **PR-3 Every PR stands alone.** It builds, passes the whole gate, carries the tests for what it adds, and leaves the main branch releasable. It can be reviewed without reading the later parts.
- **PR-4 How to split,** in this order: (1) preparatory refactoring with no behaviour change; (2) new types, errors and traits with their tests; (3) behaviour in vertical slices, one capability per PR; (4) wiring and exposure — CLI flag, route, public re-export; (5) removal of the old path. Mechanical changes — renames, moves, reformatting, regenerated code, dependency bumps — never share a PR with behavioural changes.
- **PR-5 Unfinished functionality is unreachable for users until the last part.** Code the compiler reports as unused until a later part wires it gets a scoped `#[expect(dead_code, reason = "wired in part 3/4")]`. Where unit tests already use the item, write `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`, or the expectation is unfulfilled in test builds. Either way the gate fails as soon as the code is wired, so the marker cannot be forgotten.
- **PR-6 A purely mechanical PR may exceed the limit** — only renames, moves, reformatting, regenerated code or deletion of dead code — if it contains nothing else and its description gives the exact command or steps that reproduce it.
- **PR-7 One purpose per PR.** No unrelated changes: no reformatting of untouched code, no drive-by refactoring, no opportunistic fixes, no features nobody asked for. Report what you noticed under *Out of scope*.
- **PR-8 Commits** are small and coherent. Messages follow the repository's convention; without one: an imperative subject of at most 72 characters, and a body that says why. No build artefacts, secrets or editor files.
- **PR-9** The description follows §17, and every statement in it is true.

## 17. PR description

```markdown
## What and why

## Stack
Only for a multi-PR change: part n/N, the whole plan, what this part covers, what comes next.

## Size
N changed lines outside tests (limit 1500).

## Tests
Each new behaviour and the test that specifies it; the regression test for a bug fix.
Existing tests modified or removed, each with its reason (normally: none).

## Gate
The commands you ran and their results.

## Deviations
Lint suppressions, `unsafe`, new dependencies with their justification, assumptions,
rules not followed — each with its reason. Or "none".

## Out of scope
Problems you noticed and deliberately did not touch. Or "none".
```

## 18. Done means

- [ ] Every new behaviour was specified by a test that I saw fail first. Bug fixes have a regression test.
- [ ] No existing test was deleted, ignored or loosened — or each one is listed with its reason.
- [ ] Production code has no ignored error, no panic path, no lossy cast, no unchecked index.
- [ ] Invalid states and argument mix-ups are ruled out by types wherever that is possible.
- [ ] Errors are typed with `thiserror`, narrow, and carry context and their cause.
- [ ] Public items, constants, types and fields are documented.
- [ ] No `#[allow]`. Every `#[expect]` has a reason and is listed in the PR.
- [ ] The gate is green on the final commit: fmt, clippy `-D warnings`, tests, docs.
- [ ] At most 1500 changed lines outside tests, one purpose, nothing unrelated.
- [ ] The PR description is complete and true.

## Appendix: lint baseline

For repositories without a strict lint setup (LINT-7). Introduce it in its own PR. In a workspace, use `[workspace.lints.rust]` and `[workspace.lints.clippy]`, plus `[lints] workspace = true` in every member.

```toml
# Cargo.toml
[lints.rust]
unsafe_code = "forbid"                    # "deny" in a crate that needs audited unsafe
unused_must_use = "deny"
missing_docs = "warn"
missing_debug_implementations = "warn"
unreachable_pub = "warn"
let_underscore_drop = "warn"
rust_2018_idioms = { level = "warn", priority = -1 }

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
# ERR-3, TYP-9: no panic paths
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
todo = "warn"
unimplemented = "warn"
unreachable = "warn"
indexing_slicing = "warn"
string_slice = "warn"
# ERR-2: no ignored errors
let_underscore_must_use = "warn"
unused_result_ok = "warn"
map_err_ignore = "warn"
# LINT-3: suppressions carry a reason and expire
allow_attributes = "warn"
allow_attributes_without_reason = "warn"
# hygiene
assertions_on_result_states = "warn"
clone_on_ref_ptr = "warn"
dbg_macro = "warn"
exit = "warn"
mem_forget = "warn"
print_stderr = "warn"
print_stdout = "warn"
undocumented_unsafe_blocks = "warn"
multiple_unsafe_ops_per_block = "warn"
# optional, stricter
# missing_docs_in_private_items = "warn"  # enforces DOC-2; also wants docs on private functions
# arithmetic_side_effects = "warn"        # every unchecked + - * / on integers
```

```toml
# clippy.toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-indexing-slicing-in-tests = true
allow-print-in-tests = true
```

- `warn` here plus `-D warnings` in the gate: fatal in CI, not while you are mid-edit.
- The test allowances cover `#[test]` functions and `#[cfg(test)]` modules. Free helper functions in `tests/*.rs` are still linted — have them return `Result`.
- Never enable `clippy::restriction` as a group.
- No lint sees an `if let Ok(…)` without an `Err` arm, `filter_map(Result::ok)`, a failure masked by `unwrap_or_default()`, an unjoined task or an unchecked exit status. ERR-2 still needs your eyes.
