Allow: Cargo.lock
Allow: crates/*/Cargo.toml
Allow: CHANGELOG.md
Short desc: Tags for bookmarks: store, filter, export
Implementation task for `marks`, a fictional Rust CLI that keeps bookmarks in a JSON file (`crates/marks-core`: the
model and the store; `crates/marks`: the commands). Two features, three items, one pull request per item, in the order
of their `Depends on:` lines. Every design decision is made below: do not choose another mechanism, other names or
other formats. If something is impossible as written, ask the human in your questions file before you write code.

Labels used in this file:
- `Allow:` — a path the checks before the review let the team change without asking the human (here: the lockfile,
  the crates' manifests for the version raise, the changelog). `Short desc:` — one line of at most 70 characters that
  `agent status` and the notifications show. Both only at the top, before the brief.
- `1.`, `2.` — a feature; `1.1`, `1.2` — an item of it, one pull request each. `[ ]` marks an item not done yet.
- `Depends on:` — under a feature: the features that must be merged before it starts. Under an item (indented): the
  items it builds on, for the team's plan.
- `Expected result:` — what the reviewers check the pull request against. `Checks:` — what anyone can run or look at to
  see that the item is done. Both alone on their line, at the indentation of the item's text.
- `W1`, `W2`, ... — a worker step: a small, self-contained piece of an item that the lead hands to a worker with
  `delegate`. The number only names the step; the order is given by "first", "after W2" and `‖`.
- `‖` — the steps on both sides may run at the same time, each by its own worker.
- `By: worker` — the mark the lead puts on such a step in its plan of steps.
- `K1`, `K2`, ... — a check of an item (K for "Kontrola", check): a condition that must hold before the item is handed
  over, numbered within each item.
- `RED:`, `GREEN:`, `REFACTOR:` — the prefixes of the commits of a test-first cycle.

Why: users keep hundreds of bookmarks and cannot find them again. Issue #41 asks for tags (`marks add <url> --tag rust`)
and a filter (`marks list --tag rust`); issue #57 asks for an export of one tag as an HTML bookmarks file that browsers
import.

Rules for every item:
- Tests first: a `RED:` commit with the new tests failing (stubs that compile and return a wrong value, never
  `todo!()`), then `GREEN:`, then `REFACTOR:` if needed with the tests unchanged. A test is frozen once its RED commit
  exists; a wrong test is corrected only in a `TEST-FIX:` commit of its own, named in the PR description.
- One PR per item, `Fixes #41` (items 1.x) or `Fixes #57` (item 2.1) in its description. Raise by `0.1.0` from `main`
  each crate whose code changes, in the last commit, with `Cargo.lock` and a `CHANGELOG.md` entry.
- Compatibility: a store file written by the current version (no `tags` key) loads unchanged; add it as a fixture
  before you change the model.
- No new dependency. No bare sleep in a test. Tests write only to a temporary directory.
- Run single tests while you work; the full check (`make check`: fmt, clippy with warnings denied, the tests) once,
  on the commit you hand over.
- The PR description: what and why, how it was tested (commands and results), the compatibility note, anything the
  reviewer should look at first.

Workers: give the steps below to workers with `delegate` and mark them `By: worker` in your plan. Each step is
self-contained: give the worker the text of the step, its file list and its check, nothing more. A step changes at
most two files, makes no design choice and ends with one command that tells whether it is done. Steps joined by ‖ may
run at the same time. You write the code the steps do not name (the body of `Tag::parse` and the serde wiring of 1.1,
the filter in `Store::list` of 1.2, the HTML writer and its escaping in 2.1), review every worker's commit before you
build on it, and run the full check yourself. Start item 2.1's steps while item 1.2 is in review.

- W1 (item 1.1, first) ‖ W2:
  - W1: the fixture `crates/marks-core/tests/fixtures/store-v1.json`: three bookmarks in the current format, no `tags`
    key, URLs on `example.com`. Copy the shape from `Store::save`'s output on `main`. Check: `jq . <file>` parses it.
  - W2: the type `Tag` and the enum `TagError` in a new `crates/marks-core/src/tag.rs`, as in 1.1, with `pub mod tag;`
    in `lib.rs`. Stub bodies only (`parse` returns `Err(TagError::Empty)` for every input). Check:
    `cargo build -p marks-core`.
- W3 (item 1.1, after W2) ‖ W4 (after W1):
  - W3: the RED tests of `Tag::parse` and of `TagError`'s messages in `crates/marks-core/src/tag/tests.rs`, every case
    of the list in 1.1. Commit as `RED:` with the failing output. Check: the file compiles, the new tests fail, every
    other test passes.
  - W4: the RED tests of the store in `crates/marks-core/tests/store_compat.rs`: the fixture loads, the round trip.
    Check: it compiles and fails only on the missing field.
- W5 (item 1.2) ‖ W6 ‖ W7:
  - W5: the flag `--tag <name>` (repeatable) on `marks add` and `marks list` in `crates/marks/src/cli.rs` (clap
    derive), passed through as `Vec<String>`; nothing else. Check: `cargo run -p marks -- list --help` shows the flag.
  - W6: the RED tests of 1.2's CLI cases in `crates/marks/tests/tags.rs`, every case of the list, against a temporary
    store file. Check: they compile and fail.
  - W7: `README.md`, section "Usage": the two examples of 1.2 and one sentence on the tag rules.
- W8 (item 2.1) ‖ W9 ‖ W10:
  - W8: the golden file `crates/marks/tests/golden/export-rust.html`, written by hand from the format of 2.1 for the
    two bookmarks of the test store. Check: it opens in a browser and shows a folder "rust" with two links.
  - W9: the RED tests of 2.1 in `crates/marks/tests/export.rs`, every case of the list. Check: they compile and fail.
  - W10: `README.md`, section "Usage": the example of 2.1 and one sentence on `--force`.
- W11 (last of each PR): the `CHANGELOG.md` entry and the version raises with `Cargo.lock`, from the versions on
  `main`. Check: `cargo build --locked` passes.

Not in this task: renaming or deleting a tag across all bookmarks; tags in the import command; colours.

Estimate without tests (mine + 50%): 1.1 — 120 lines; 1.2 — 150 lines; 2.1 — 180 lines.

1. Bookmarks carry tags (#41)
Depends on: none
[ ] 1.1 The model stores tags and old store files still load
    Depends on: none
    Expected result:
    - `crates/marks-core/src/tag.rs`: `Tag`, a newtype over `String`, with `Tag::parse(&str) -> Result<Tag, TagError>`,
      `Display`, `Ord`, and `Serialize`/`Deserialize` through `parse` (a stored tag that does not parse fails the load
      with the line of the file).
    - `Tag::parse` trims the input, lowercases it, and accepts 1 to 32 characters of `a-z`, `0-9`, `-`; otherwise
      `TagError::Empty`, `TagError::TooLong { len }` or `TagError::InvalidChar { ch }` (the first offending character).
    - `TagError` displays as `empty`, `longer than 32 characters (<len>)` and `character '<ch>' is not allowed`.
    - `crates/marks-core/src/bookmark.rs`: `Bookmark` gains `tags: BTreeSet<Tag>` with `#[serde(default)]`, so a file
      without the key loads with no tags. Saving always writes the key.
    - `CHANGELOG.md`: under "New features", "Bookmarks can carry tags (stored only; the commands come next)".
    - Tests, new:
      - `parse("Rust")` → `rust`; `parse("  web-dev ")` → `web-dev`; `parse("")` and `parse("   ")` → `Empty`;
        33 × `a` → `TooLong { len: 33 }`; 32 × `a` → ok; `parse("c++")` → `InvalidChar { ch: '+' }`;
        `parse("zażółć")` → `InvalidChar { ch: 'ż' }`;
      - the three messages of `TagError`, exactly as above;
      - `store-v1.json` loads: three bookmarks, every `tags` empty; saved again, it has `"tags": []` on each;
      - a bookmark with tags `[web, rust]` round-trips through save and load as `{rust, web}`;
      - a store file with the tag `"C++"` fails to load with an error that names the tag.
    Checks:
    - K1. No existing test changes in this item.
    - K2. `marks-core` raised by `0.1.0`, with `Cargo.lock` and the `CHANGELOG.md` entry; `make check` passes.
[ ] 1.2 `marks add --tag` and `marks list --tag`
    Depends on: 1.1
    Expected result:
    - `marks add <url> --tag a --tag b` stores the bookmark with both tags; the same tag twice (in any case) is stored
      once. An invalid tag refuses the whole command with exit 2 and `error: invalid tag "<input>": <TagError>`;
      nothing is saved.
    - `marks list --tag a --tag b` prints only bookmarks that have **all** the given tags, in the order of the store.
      Without `--tag`, output is exactly as today. Each listed line of a bookmark with tags ends with ` [a, b]`, the
      tags sorted; a bookmark without tags is printed as today.
    - `crates/marks-core/src/store.rs`: `Store::list(&self, filter: &[Tag]) -> Vec<&Bookmark>`; an empty filter
      returns everything.
    - `README.md`: the examples of W7. `CHANGELOG.md`: "`marks add --tag` and `marks list --tag`".
    - Tests, new (`crates/marks/tests/tags.rs`):
      - add with `--tag Rust --tag cli` → the store has tags `{cli, rust}`;
      - add with `--tag rust --tag RUST` → the store has the tag `rust` once;
      - add with `--tag "c++"` → exit 2, stderr `error: invalid tag "c++": character '+' is not allowed`, the store
        file is byte-for-byte unchanged;
      - store with A `{rust}`, B `{rust, cli}`, C `{}` → `list --tag rust` prints A, B; `--tag rust --tag cli` prints
        B; `--tag go` prints nothing and exits 0; no `--tag` prints A, B, C, the line of C exactly as on `main`;
      - the line of B ends with ` [cli, rust]`;
      - `list --tag "c++"` → exit 2, the same message as `add`.
    Checks:
    - K1. No existing test changes in this item.
    - K2. `marks` and `marks-core` raised by `0.1.0`, with `Cargo.lock` and the `CHANGELOG.md` entry; `make check`
      passes.

2. Export one tag for a browser (#57)
Depends on: 1
[ ] 2.1 `marks export --tag <name> --html <file>`
    Depends on: 1.2
    Expected result:
    - A new subcommand `export` in `crates/marks/src/export.rs`. It writes the Netscape bookmark file format that
      browsers import: the `<!DOCTYPE NETSCAPE-Bookmark-file-1>` header, one `<H3>` folder named after the tag, one
      `<DT><A HREF="..." ADD_DATE="<unix seconds>">title</A>` per bookmark, in the order of the store, UTF-8, `\n`
      line ends.
    - Titles and URLs are HTML-escaped (`&`, `<`, `>`, `"`). An existing file is not overwritten unless `--force`;
      without it: exit 2, `error: <file> exists; use --force to replace it`. An invalid tag: exit 2, the message of 1.2.
    - stdout: `exported <n> bookmarks to <file>`. No bookmark with the tag: the file is still written with an empty
      folder and `n` is 0.
    - The file is written to a temporary file in the same directory and renamed, so a failed export leaves no half
      file.
    - `README.md`: the example of W10. `CHANGELOG.md`: "`marks export --tag <name> --html <file>`".
    - Tests, new (`crates/marks/tests/export.rs`):
      - the test store, `export --tag rust --html out.html` → `out.html` equals `golden/export-rust.html`, stdout
        `exported 2 bookmarks to out.html`;
      - a title `Tom & Jerry <3` → written as `Tom &amp; Jerry &lt;3`; a URL with `"` → `&quot;`;
      - the target exists → exit 2, the file unchanged; with `--force` → replaced;
      - `--tag go` → an empty folder, stdout `exported 0 bookmarks to out.html`, exit 0;
      - `--tag "c++"` → exit 2, no file written;
      - the target directory is read-only → exit 1, no file and no temporary file left.
    Checks:
    - K1. No existing test changes in this item.
    - K2. `marks` raised by `0.1.0`, with `Cargo.lock` and the `CHANGELOG.md` entry; `make check` passes.
    - K3. The golden file imports in Firefox or Chromium as one folder with two links (the lead tries it once and says
      so in the PR description).
