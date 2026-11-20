# The plan of a task

Every task keeps a plan of its steps in its worktree, at **`.agent/plan.md`**: what the task is for, the stages it is
done in, and the steps, each with the check that shows it is done. A plan belongs to the task, not to the harness: the
agent writes and keeps it up to date, and the harness reads the file only, so a task that was interrupted can be
started again from what the plan says is left. The reader is [`parse_plan`](../crates/agent-core/src/plan.rs); nothing
calls it yet.

The plan is not only for code. A team may write a document or make a set of images, and the three examples below show
one of each.

## Format

```text
Finish the guide, then publish it.           the intent: every line before the first stage

1. Draft                                    a stage: "N. Title" at column 0
[ ] 1.1 Write the outline                   a step: "[mark] N.M Title"
    Files: docs/plan.md, src/a.rs            at most one: the paths the step is about
    Check: exists docs/plan.md              at most one: how it is shown to be done
    Start with the audience.                the rest of the text is the step's note
[>] 1.2 Write the first draft
2. Publish
[x] 2.1 Add the guide to the release
    Check: cargo test -p agent-core
[-] 2.2 Translate it                        given up: its note must say why
    Nobody asked for a translation.
```

The lines follow the [task list](task-list.md)'s format, read in a plan mode: what the task list calls a feature is a
stage here, and what it calls an item is a step. A plan has no `Coordinator:` or `Allow:` header — a `Coordinator:` or
`Allow:` line at column 0 inside a stage is text at column 0, which the format refuses there.

### The lines

- **The intent** — every line before the first stage, without the blank lines at its start and end, and otherwise
  kept as written, indentation included. A plan may have none.
- **A stage** — `N. Title` at column 0: a number from 1 to 65535 without leading zeros, a `.`, a space or tab, a
  title. Numbers are unique, but a plan need not number its stages 1, 2, 3 — a stage number must not be reused by a
  later stage. **A numbered line at column 0 therefore starts a stage even in prose**: indent a numbered list in the
  intent, or use `-`.
- **A step** — `[mark] N.M Title` at column 0, in the stage whose number is `N`: a mark, a space, a step ID (two
  numbers without leading zeros), a space, a title. A step's ID is what names it for ever: a later version of the
  plan never gives an ID to another step, and a step that is no longer done is `[-]`, not a line that disappears.
- **The state of a step** — the mark:

  | Mark | State | Means |
  |---|---|---|
  | `[ ]` | to do | not started |
  | `[>]` | in hand | being worked on; **at most one step of the whole plan** |
  | `[x]` or `[X]` | done | finished, and its check shows it |
  | `[-]` | given up | not done and not to be done: the note says why |

- **The text of a step** — the indented lines after the step's line, until the next line at column 0. Its smallest
  indentation is removed; relative indentation, blank lines inside and non-ASCII text are kept.
- **`Files:`** — at most one line of the text, `Files:` and then the paths the step is about, separated by commas; the
  list does not go on to the next line, which would be note text. Every entry is a path: `Files:` with nothing after
  it, or an empty entry (`a.rs,,b.rs`, a comma at the end), is refused. A path is relative to the repository's root, by the
  rules of the task list's `Allow:` paths without globs: no absolute path, no empty segment, no `.` or `..` segment, no
  control character; `*` and `?` are characters of a file's name, not patterns. `Files:` and `Check:` count only at
  the start of a line of the text, after its indentation.
- **`Check:`** — at most one line of the text: exactly one of the three below. A step without a `Check:` line is
  valid — a step whose being done nobody can show by a machine or by a path is written without it.
- **The note** — every other line of the step's text. A `[-]` step needs a note that is not blank: its reason. A note
  that is only a `Files:` line or a `Check:` line is no reason, and is refused as none.

Two words of a `Check:` line are not commands, and how a command is told from them:

| `Check:` line | The check |
|---|---|
| `reviewer` exactly, with nothing after it | a judgement: a person reads the result and says whether the step is done. Never a command |
| `exists PATH`, with a path after it | the path is there when the step is done; `PATH` follows the rules of a `Files:` path |
| `exists` with nothing after it | refused (`ExistsWithoutPath`) |
| any other line, `existsX` included | a command, run later by the daemon in the task's worktree; one line |

The line after `Check:` is trimmed and then told apart: `reviewer` counts only when it is the whole line, so
`reviewer` followed by anything — `reviewer says it is fine` — is a command; `exists` counts when it is the whole line
or the first word, followed by a space or a tab and the path, which may itself contain spaces. A command is one line: there is no way to write a check of several lines.

Line ends may be `\n` or `\r\n`, the last line needs no newline, and a byte order mark at the start of the file is
ignored. A plan may be 1 MiB at most, the task list's limit — as it is written and as it is written back: the
harness writes a plan back with the text of its steps indented by four spaces, and a plan whose text written back
would pass 1 MiB is refused, so that every plan read can be written back and read again.

## Errors

The first fault in the text is the one reported, and every error but the empty text and the two over the size limit
names the number of its line, counted from 1 over the whole file (the byte order mark, blank lines and the intent
included). A fault in the text of a step is reported before a broken line below it: the plan is read in the order of
its faults.

| Error | When |
|---|---|
| `Empty` | nothing in the text but blank lines |
| `TooLarge` | the text is over 1 MiB |
| `TooLargeWrittenBack` | the text is within 1 MiB, but the plan written back is over it |
| `StageNumber` | the number of a stage is `0`, has a leading zero, or is over 65535 |
| `StageWithoutTitle` | `N.` with no title |
| `DuplicateStage` | a stage number is used again (the first line is in the error) |
| `MalformedStep` | a step line whose mark is not one of the four, whose mark is not followed by a space, whose ID is invalid or whose title is missing |
| `StepBeforeStage` | a step before the first stage |
| `StepInOtherStage` | a step whose number is not that of the stage it is under |
| `DuplicateStep` | a step ID is used again (the first line is in the error) |
| `TwoInHand` | a second step is in hand (the first one's line is in the error) |
| `TextBeforeStep` | an indented line in a stage before its first step |
| `UnexpectedLine` | a line at column 0 inside a stage that is not a stage or a step |
| `GivenUpWithoutReason` | a `[-]` step whose note is blank |
| `BadFile` | a `Files:` path that is empty, absolute, has `..`, or is otherwise invalid (the cause in the error) |
| `DuplicatePart` | a second `Files:` or `Check:` line (the line of the second) |
| `EmptyCheck` | a `Check:` line with nothing after it |
| `ExistsWithoutPath` | `Check: exists` with no path after it |
| `BadCheck` | an `exists` check whose path is invalid (the cause in the error) |

## Comparing two versions

Two plans are compared step by step, **by ID and never by text**: a step that keeps its ID is the same step, whatever
its title says, and a title it takes on is reported on its own. `Plan::compare` gives five things:

- the steps **added** — their IDs are in the newer version and were not in the older one;
- the steps whose **state changed**, with the state before and after;
- the steps that **disappeared** — the older version had their ID and the newer one does not. A step is never to
  disappear: the answer to it is to give it up (`[-]`) with its reason, and a comparison that reports one is telling
  the reader that the plan broke this rule;
- the steps whose ID the newer version uses for **another title**;
- by `is_empty`, whether none of these happened, and by `loses_a_step`, whether a step disappeared or its ID got
  another title.

## The three examples

Each example below is a file of `crates/agent-core/tests/fixtures/` and a test there reads this page: the fenced
`text` block of each example appears here as it is in the fixture, and both parse.

### A code change

```text
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
```

### A document

```text
Write the operator's guide for the new daemon settings: one page a reader can follow from a fresh checkout to a
running daemon.

1. Draft
[x] 1.1 List the questions a new operator asks
    Check: reviewer
[>] 1.2 Write the walkthrough
    Files: docs/operator-guide.md
    One section per question, each with the commands to type and what they print.
    Check: exists docs/operator-guide.md
[ ] 1.3 Check every command on a fresh machine
    Files: docs/operator-guide.md
    Check: reviewer
2. Review
[ ] 2.1 Have somebody who did not write it follow it
    Check: reviewer
[-] 2.2 Translate it into German
    The human asked for English only for now.
    Check: reviewer
```

### A set of images

```text
Make the six screenshots of the dashboard the README shows, in the dark theme and at 1440 pixels wide.

1. Capture
[x] 1.1 Start the dashboard from the example configuration
    Files: config/agents.example.toml
    Check: exists config/agents.example.toml
[>] 1.2 Capture the six screens
    Files: docs/images/dashboard.png, docs/images/dashboard-teams.png, docs/images/dashboard-rounds.png, docs/images/dashboard-history.png, docs/images/dashboard-notebook.png, docs/images/dashboard-settings.png
    No window border, no cursor, the same browser window each time.
    Check: reviewer
[ ] 1.3 Check the text on each image is readable without zooming
    Check: reviewer
2. Publish
[ ] 2.1 Put the images in the README next to what they show
    Files: README.md, docs/images/dashboard.png, docs/images/dashboard-teams.png
    Check: reviewer
```
