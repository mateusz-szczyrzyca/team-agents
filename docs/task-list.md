# The task list

`agent run <agents...> "tasks"` and `agent run <agents...> -f tasks.md` start work from a **task list**: a text with an
optional coordinator, a brief, and features made of items. One item is one task, one branch and one pull request. The
text of `"tasks"` and the text of the file are parsed by the same function, so they give the same list. The plan of the
Rust rewrite itself, [`rust-agent-plan.md`](../crates/agent-core/tests/fixtures/rust-agent-plan.md), is a task list and a
test fixture: it must parse. The parser is [`agent-core`](../crates/agent-core/src/task_list.rs)'s `TaskList::parse`.

## Format

```text
Coordinator: inv-opus                       optional, the first line of the text

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
- **The brief** — every line before the first feature, without the `Coordinator:` line, with the blank lines at its
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
| `InvalidCoordinator` | the `Coordinator:` line has no valid name |
| `FeatureNumber` | the number of a feature is `0`, has a leading zero, or is over 65535 |
| `FeatureWithoutTitle` | `N.` with no title |
| `DuplicateFeature` | a feature number is used again (the first line is in the error) |
| `MalformedItem` | a line at column 0 inside a feature that starts with `[` is not a valid item line; `ItemProblem` says why: `BadMark` (not `[ ]` or `[x]`), `NoSpaceAfterMark`, `BadId` (the item number is invalid), `MissingTitle` |
| `ItemBeforeFeature` | an item (`[ ]` or `[x]`) before the first feature; other lines starting with `[` in the brief are text |
| `ItemInOtherFeature` | `2.1` under feature 1 |
| `DuplicateItem` | an item number is used again (the first line is in the error) |
| `MalformedDependsOn` | `Depends on:` is not `none` or feature numbers |
| `UnknownDependency` | `Depends on:` names a feature that is not defined earlier (itself included) |
| `MisplacedDependsOn` | `Depends on:` after an item, or a second one |
| `TextBeforeItem` | an indented line in a feature before its first item |
| `UnexpectedLine` | a line at column 0 inside a feature that is not a feature, an item or `Depends on:` — indent it under its item |
| `DuplicatePart` | a label twice in one item (the line of the second) |
| `EmptyPart` | a label with no text after it |

## What the parser gives

```text
TaskList { coordinator: Option<AgentName>, brief, features }
Feature  { number, title, depends_on: Vec<feature number>, items }
Item     { id: ItemId, title, mark: Open | Done, description, expected_result: Option, checks: Option }
```

`TaskList` is made only by `parse`, so its parts are consistent: unique numbers, every item in its feature, dependencies
only on earlier features. It does not know which features are merged — that is progress, kept in the state (item 2.2) —
and it is not persisted: the state stores what it needs from it.
