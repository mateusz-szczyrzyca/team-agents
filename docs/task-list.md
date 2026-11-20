# The task list

`agent run <agents...> "tasks"` and `agent run <agents...> -f tasks.md` start work from a **task list**: a text with an
optional coordinator and path allowances, a brief, and features made of items. One item is one task, one branch and one pull request. The
text of `"tasks"` and the text of the file are parsed by the same function, so they give the same list. The plan of the
Rust rewrite itself, [`rust-agent-plan.md`](../crates/agent-core/tests/fixtures/rust-agent-plan.md), is a task list and a
test fixture: it must parse. The parser is [`agent-core`](../crates/agent-core/src/task_list.rs)'s `TaskList::parse`.

`--allow-unpushed-base` is a separate command-line permission, not a task-list directive. It permits a team to
start with local base commits absent from `origin`; the harness retains their count and first commit and includes
them in every pull request. `Allow:` path allowances do not grant this permission.

## Format

```text
Coordinator: inv-opus                       optional, the first line of the text
Allow: Cargo.toml                           optional paths allowed before every round
Allow: tests/**/*.golden

Rewrite the thing. Everything before the    the brief: free text every member reads
first feature is the brief.

1. First feature                            a feature: "N. Title" at column 0
Depends on: none                            optional: "none" or feature numbers, right after the feature line
[ ] 1.1 First item                          an item: "[ ] N.M Title" ("[x]" for done)
    What to do, in indented lines.          the description
    Expected result:                        a part: its label alone on a line
    - Files: `src/a.rs`.                    what the reviewers check the pull request against
    Checks:                                 another part
    - `cargo test` passes.                  what anyone can run to see that it works
[x] 1.2 Second item, already done

2. Second feature
Depends on: 1
[ ] 2.1 Third item
```

### The lines

- **`Coordinator: NAME`** — only as the first line that is not blank. `NAME` is an agent's or an investigator's name
  (both have the same form); which of them it is, the configuration says. `--coordinator` on the command line wins over
  it. Later in the brief the line is plain text (inside a feature it is an `UnexpectedLine`). An invalid name is an
  error at that line.
- **`Allow: PATH`** — at column 0, before the first nonblank line of the brief and after `Coordinator:` when
  present. Repeat it for each path or glob the task requires changing. Paths are relative to the repository root;
  `*` matches characters within one path component, `?` one character, and a whole `**` component matches zero or
  more components. `Cargo.toml` covers only the root file; `tests/**/*.golden` covers golden files at every depth
  under `tests/`. Whitespace around the pattern is removed. Empty patterns, absolute paths, `..`, unsupported glob
  syntax, and patterns that could reach `AGENTS.md`, `CLAUDE.md` or `.github/workflows/` are errors naming their line.
  Consequently `*`, `**`, `**/*`, `*.md`, `tests/**`, `.github/**` and `**/CLAUDE.md` are refused;
  recursive directory globs could cover a nested rules file. Patterns are at most 512 bytes,
  with at most four `**` components. After the brief starts an `Allow:` line is ordinary brief text.
  The team keeps these allowances for every round and daemon restart. They cover changes, removals and renames of
  test files and protected configuration paths; uncovered paths still require a decision. They never permit lint
  suppressions, skipped tests, test-detection code or missing tests. Later human answers remain per-path decisions.
  `agent run` prints every allowance. Each pull request lists changed covered paths and their `Allow:` line under
  the harness-owned `## Allowed beforehand`; no covered changes means no heading. Reviewers judge those changes
  on their behavior. A text containing only header directives is empty.
- **The brief** — every line before the first feature, without the header directives, with the blank lines at its
  start and end removed and everything else kept, indentation included. A text with no feature at all is a brief
  alone, which is how `agent run claude-1 "Fix the flaky test"` works. Text that is only blank lines (or only a
  coordinator) is an error.
- **A feature** — `N. Title` at column 0: a number from 1 to 65535 without leading zeros, a `.`, a space or tab, a
  title. Numbers are unique but need not be consecutive. **A line like `1. first` at column 0 therefore starts the
  first feature even in prose**: indent a numbered list in the brief, or use `-`.
- **`Depends on:`** — at column 0, right after its feature's line (blank lines in between are fine), at most once.
  Either `none` (any case) or feature numbers separated by commas or spaces (`1`, `2, 3`, `2 3`). Every number must be
  a feature defined *earlier* in the text, so dependencies can never form a cycle. The features it names must be merged
  before this feature starts. Items of a feature are taken in the order written. Without the line a feature waits for
  nothing.
- **An item** — `[ ] N.M Title` or `[x] N.M Title` (`[X]` too) at column 0, in the feature `N`: the mark, a space, an
  item number (`2.3`, as `ItemId`: two numbers without leading zeros), a space, the title. Item numbers are unique.
  `[x]` means the author says it is done; it is only a mark on the item (`Item::mark`).
- **The description** — the indented lines (spaces or tabs) after the item line, until the next line at column 0. The
  smallest indentation of the item's lines is removed; relative indentation, blank lines inside, and non-ASCII text are
  kept; blank lines at the start and the end are dropped.
- **The parts** — `Expected result:` and `Checks:`, each **alone on its line, at the same indentation as the item's
  first line of text**, start a part that runs to the next label or the end of the item. A label elsewhere —
  with text after it on the line, deeper or shallower, in a bullet, in another case — is ordinary text. A part with no
  text, and a label used twice in one item, are errors. The two parts may come in either order and are both optional.
  `Item::expected_result` is what the reviewers check the pull request against; `Item::checks` is what they run.

Line ends may be `\n` or `\r\n`, the last line needs no newline, and a byte order mark at the start of a file is
ignored. A text may be 1 MiB at most.

## Errors

Every error but the first two carries the number of its line, counted from 1 over the whole text (the byte order
mark, blank lines and the coordinator line count). The first fault in the text is the one reported. `TaskListError`:

| Variant | When |
|---|---|
| `Empty` | nothing in the text but blank lines, or only a `Coordinator:` line |
| `TooLarge` | the text is over 1 MiB |
| `InvalidAllowance` | an `Allow:` header has invalid or unsafe syntax; the error names its line and pattern |
| `InvalidCoordinator` | the `Coordinator:` line has no valid name |
| `FeatureNumber` | the number of a feature is `0`, has a leading zero, or is over 65535; `NumberProblem` says which: `Shape` (not digits, or a leading zero) or `Range` (`0`, over 65535) |
| `FeatureWithoutTitle` | `N.` with no title |
| `DuplicateFeature` | a feature number is used again (the first line is in the error) |
| `MalformedItem` | a line at column 0 inside a feature that starts with `[` is not a valid item line; `ItemProblem` says why: `BadMark` (not `[ ]` or `[x]`), `NoSpaceAfterMark`, `BadId` (the item number is invalid), `MissingTitle` |
| `ItemBeforeFeature` | an item (`[ ]` or `[x]`) before the first feature; other lines starting with `[` in the brief are text |
| `ItemInOtherFeature` | `2.1` under feature 1 |
| `DuplicateItem` | an item number is used again (the first line is in the error) |
| `MalformedDependsOn` | `Depends on:` is not `none` or feature numbers; the `NumberProblem` of the first word that is not one, none when there is no word |
| `UnknownDependency` | `Depends on:` names a feature that is not defined earlier (itself included) |
| `MisplacedDependsOn` | `Depends on:` after an item, or a second one |
| `TextBeforeItem` | an indented line in a feature before its first item |
| `UnexpectedLine` | a line at column 0 inside a feature that is not a feature, an item or `Depends on:` — indent it under its item |
| `DuplicatePart` | a label twice in one item (the line of the second) |
| `EmptyPart` | a label with no text after it |

## What the parser gives

```text
TaskList { allowances: Vec<TaskAllowance>, coordinator: Option<AgentName>, brief, features }
Feature  { number, title, depends_on: Vec<feature number>, items }
Item     { id: ItemId, title, mark: Open | Done, description, expected_result: Option, checks: Option }
```

`TaskList` is made only by `parse`, so its parts are consistent: unique numbers, every item in its feature, dependencies
only on earlier features. It does not know which features are merged — that is progress, kept in the state (item 2.2) —
and it is not persisted: the state stores what it needs from it.

## Approved split plans: core format

Only the coordinator requests a split, in one questions file with the full proposed verdict as an example in the
question. The human approves the whole plan using the form below. `agent answer` reports the recorded parts, or
explains why none were recorded. An ordinary `ok` does not approve the example implicitly.

Inside `**Verdict:**`, write `split` on its own line immediately followed by one fenced `json` object:

````markdown
**Verdict:**
split
```json
{
  "parent": "1.1",
  "parts": [
    {
      "title": "Draft the guide",
      "delivers": "A guide with worked examples",
      "checks": "A reviewer follows each example",
      "depends_on": []
    },
    {
      "title": "Publish the guide",
      "delivers": "The reviewed guide in the release bundle",
      "checks": "The release bundle contains the guide",
      "depends_on": ["a"]
    }
  ]
}
```
````

All fields are required; additional fields are errors. There must be 2–26 parts, each with a nonblank title,
deliverable and checks. Checks are criteria for any independently deliverable work, not necessarily code or tests.
Keys are assigned in declaration order: the example's parts are `1.1a` and `1.1b`. Dependencies name single lowercase
letters within that plan. Forward references are allowed; duplicate dependencies, unknown parts and cycles are errors.
The reader rejects the whole invalid plan and retains its cause; it never returns some of its parts.
Only the exact marker and JSON fence begin an explicit approval. A free answer such as `split` or
`please split into two parts` carries no recorded plan. `ok` does not copy a proposal from the recommendation.

The task-list parser still accepts only two-number item IDs such as `1.1`; lettered part identities belong to the
stored split plan. Existing items, branch names, rounds and histories keep their identities. The pure core decision
returns unmerged parts whose dependencies have all merged, in declaration order; only all parts merged permits
completion of the parent. The current working round becomes part a and keeps its original names; its first part
must have no dependencies. Each later part gets its own round, branch and pull request, assigned to the same team
through its usual assignment process. The daemon picks the first dependency-ready part in declaration order.
Its prompt identifies that part's deliverable and checks, excludes the other parts, and names earlier merged pull requests. The parent stays
open until every part merges, including with `--no-merge`. Plans and part progress survive daemon restarts.
`agent status` lists every part as done, in progress, waiting for named dependencies, or to do.
Assignment and one-open-team-per-agent rules remain unchanged.
