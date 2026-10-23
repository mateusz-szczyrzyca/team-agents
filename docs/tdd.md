# Test first — how agents (and people) change code here

Every change of behaviour starts with a test that fails for the right reason. That gives two things: the reviewer
reads the tests as the specification before the code, and the history shows that each test really guarded the
behaviour it claims to guard (it failed before the code existed).

The worked example is the round size counter, `agentbus/roundsize.go` and `agentbus/roundsize_test.go`. Read its
history in order — `git log --reverse --format='%h %s%n%b' -- agentbus/roundsize*.go`:

| Commit | What it shows |
|---|---|
| *Round size counter: tests first (red)* | the specification as tests; signatures with zero-value bodies; the failing run in the message |
| *…: the simplest code that passes (green)* | just enough code; the passing run in the message |
| *…: name the pattern kinds (refactor)* | better code (a typed enum instead of string checks), the test file untouched |
| *Round size of a repository: tests first (red)* | the second cycle, for the thin layer that runs git — including a bug found while thinking about the code (a base starting with `-`), written as a test first |
| *Round size of a repository and agentbus round-size (green)* | the implementation |

## The cycle

1. **RED.** Write the test. Run it; it must fail on an assertion about the behaviour. Commit the test (with whatever
   bare signatures it needs) on its own. The commit message ends with the failing run:

   ```
   RED: go test -run 'Numstat|CountRound' ./agentbus
   --- FAIL: TestCountRound (0.00s)
       roundsize_test.go:142: countRound: got {…} want {…}
   ```

2. **GREEN.** The simplest code that makes the test pass — no more. Commit it; the message ends with
   `GREEN: <command>` and the passing lines.
3. **REFACTOR.** Improve names, types, structure. The tests do not change (check:
   `git diff --stat -- '*_test.go'` is empty) and still pass.

The RED commit is the one commit allowed to fail — only on its new tests; everything else builds.

## In Go

- **Signature first, zero-value body.** `func countRound(...) RoundSize { return RoundSize{} }` compiles, so the test
  fails on an assertion. A compile error proves nothing about the behaviour, and `panic("TODO")` aborts the whole
  test binary instead of failing one test.
- **One rule per test, stated in its name and comment** (`// Rule: excluded and binary files are listed, not
  counted.`). Table-driven cases for the variations of that rule.
- **Errors by their cause, never by their message:**

  ```go
  var e *CmdRoundSizeError
  if !errors.As(err.asError(), &e) || e.Cause != CmdRoundSizeBadNumstat { … }
  ```

  The cause enum and the error type come from `errors.json` (`make generate`); add the cause in the RED commit, so
  the test compiles.
- **A pure core, a thin shell.** The counter is a function of git's output (bytes in, a value out) and is tested
  without git. Only the thin layer runs git — once, in a temporary repository, isolated from the user's
  configuration (`GIT_CONFIG_GLOBAL=/dev/null`, `commit.gpgsign=false`): a signing key or a hook on the machine must
  not decide whether a test passes.
- **Fakes at process boundaries.** Calls of external programs go through package variables (`callAgent`,
  `callTailscale` in `live/`). Tests replace them; `TestMain` sets them to fail loudly, so no test can run the real
  installation by accident.
- **Never the real `~/agents`.** `t.TempDir()`, `newBus(t)`, `httptest` servers on loopback.
- **A bug you notice becomes a test first** — then the fix.
- **Concurrency:** run `go test -race`.

## In bats (`bin/agent`)

The same cycle: the tests in `tests/unit/*.bats` first (stub CLIs in `tests/unit/stubs/`, the fake agent driven
by `scenario`), the script second. `tests/unit/rounds.bats` and its commits are the example.

## When a test first is not possible

Layout and CSS, prompt wording, a line of configuration, a fix of a race in an existing test. Say so in the commit
message — `Test-First: not possible — <reason>` — so the reviewer does not have to guess. A change of behaviour with
neither a test commit before it nor that line is a **major** review finding.

## Tests ahead of their round

A test for functionality of a later round may be merged earlier, skipped with a marker naming the round:

```go
t.Skip("pending: board_next ordering, round 4")
```

Round 4 must remove it; the script sends a round back while a test due in it is still pending.
