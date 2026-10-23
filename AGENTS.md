# AGENTS.md — team-agents

Instructions for AI agents (and humans) working on this repository. They are binding. Read the whole file
before you start; the [README](README.md) explains how the system is used.

## What this repository is

team-agents runs headless AI coding agents — Claude Code, Codex, Antigravity (Gemini) and OpenCode — on a Linux
host, alone or as teams, under `systemd --user`. It does not implement an agent loop: it starts the agent CLIs,
gives them a protocol (the prompts), a shared bus for coordination (an MCP server) and a human in the loop
through question files and notifications.

**This repository is developed mostly by its own agents.** You are probably running inside a team-agents
installation right now, started by the same `agent` script you are about to change. Keep that in mind (see
"Never" below).

### Components

| Component | Where | Language | Role |
|---|---|---|---|
| `agent` | `bin/agent` | Bash | the CLI and the runner: tasks, teams, git worktrees, systemd units, prompts, watchdog, notifications, archives |
| `agentbus` | `agentbus/` | Go, standard library only | the MCP server every agent run talks to (messages, reservations, investigations, teams, board, notebook, checkpoints) and CLI subcommands the script uses; shared state in `~/agents/bus/` under `flock` |
| `agentlive` | `live/` | Go + Bubble Tea; the web interface: Go standard library + vendored htmx | the terminal panel `agent live`, and `agent web` (`live/web*.go`, templates and static files in `live/web/`; never edit `live/web/static/vendor/` by hand — `tools/update-web-vendor.sh`) |
| prompts | `prompts/*.md` | Markdown | the protocol given to the agents; `{{PLACEHOLDERS}}` are filled by `build_prompt` in `bin/agent` |
| `errgen` | `tools/errgen/` | Go | generates the per-operation error types `agentbus/errors_gen.go` from `agentbus/errors.json` |
| `doccheck` | `tools/doccheck/` | Go | fails when a constant, variable, type or struct field has no comment |
| tests | `agentbus/*_test.go`, `live/*_test.go`, `tests/unit/` (bats with stub CLIs), `tests/integration/` (real CLIs, opt-in) | | |

### How the parts talk

`agent run` creates a task (directory `~/agents/tasks/<id>/` with `meta.env`), a git worktree and a branch,
and starts a transient systemd unit running `agent _runner`. The runner builds the prompt, starts the agent CLI
with an MCP configuration that spawns `agentbus serve` for that run, normalizes the CLI's event stream into
`activity.jsonl` (`agentbus normalize`) and acts on the `STATUS:` line of the agent's final message. Every
`agentbus` process reads and writes the same `bus/state.json` and `bus/messages.jsonl` under an exclusive lock.
The script calls `agentbus` subcommands (`team-get`, `inv-next`, …) and relies on their output format and exit
codes.

## Never

These rules have no exceptions unless the human explicitly asks for it in the task.

1. **Never modify existing tests.** Do not change, delete, rename, skip or weaken any existing test, test helper
   or stub behaviour (`*_test.go`, `tests/**`), and do not change expected values to make a test pass. When a
   test fails, the code is wrong until proven otherwise. If the task requires behaviour that an existing test
   forbids, stop and ask the human in your questions file; wait for the answer. **Adding new tests is required**
   (see "Tests"); adding a new action to a stub CLI is allowed, changing an existing one is not.
2. **Never change `AGENTS.md` or `CLAUDE.md`.** Record what you learned in the repository notebook
   (`notebook_append`) and suggest changes to this file in your report.
3. **Never stop, restart or reinstall the running installation.** No `agent stop`, `agent stop --running`,
   `install.sh`, `systemctl --user stop|restart agent-*`, no edits in `~/agents/` and no `make install`-like
   commands against your own home. You and the other agents run inside it. Test only in isolated temporary
   homes — the bats suite does that for you.
4. **Never run real agent CLIs or `make integration`** unless the task asks for it: they cost the human money
   and quota.
5. **Never edit `agentbus/errors_gen.go` by hand.** Edit `agentbus/errors.json` and run `make generate`.
6. **Never change `.github/workflows/`.** You do not have permission, and CI runs with secrets. Propose changes
   in the PR description.
7. **Never commit secrets** — tokens, keys, `secrets.env`, real host names or e-mail addresses of the human. Test
   data uses `example.com` and invented names.
8. **Never add a dependency** (Go module, system package used by the script) without the human's approval.
   `agentbus` uses the standard library only.
9. **Never break compatibility silently.** These are interfaces with existing installations: the files in
   `~/agents/` (`bus/state.json`, `bus/messages.jsonl`, `tasks/*/meta.env`, `activity.jsonl`, checkpoints,
   notebooks), the output and exit codes of `agentbus` subcommands that `bin/agent` parses, the `agent` CLI,
   the configuration variables in `agents.conf`, the MCP tool names and arguments, the log event names. A change
   to any of them needs backward compatibility (old data still readable) or a migration, tests for both, and a
   README update. Say it in the PR description. `agentbus/testdata/state-legacy.json` is a bus state written
   by an earlier version: it must keep loading (`TestLegacyStateLoads`). When you change a persisted type, add a
   fixture of the format before your change and a test that loads it. Remember that stricter validation of an
   old value is a breaking change too: one rejected value makes the whole bus state unreadable.

## Setup

- Go (the version in `agentbus/go.mod` or newer), `bash` ≥ 5, `jq`, `git`, `bats`, `7z` (`p7zip-full`),
  `ssh-keygen` (`openssh-client`).
- Linters, pinned and checksum-verified: **`make tools`** installs `golangci-lint`, `staticcheck` and
  `shellcheck` into `~/.local/bin` without sudo. If `make lint` says a linter is missing, run `make tools` —
  do not skip the check and do not install other versions. Missing system packages: install them only if the
  prompt says you have sudo; otherwise report what is missing.
- The `live` module downloads its dependencies (Bubble Tea) through the Go module proxy on first build.

## Workflow and checks

Before **every** commit:

```sh
make lint test
```

- `make lint` — `shellcheck -S warning` on every Bash script, `gofmt`, `golangci-lint` (configuration:
  `.golangci.yml`) and `staticcheck` on every Go module, `doccheck`, and a check that `errors_gen.go` matches
  `errors.json`.
- `make test` — `go vet` and `go test` for `agentbus` and `live`, and the bats suite (`tests/unit`, stub CLIs,
  no network, no real accounts).
- A single Go test: `cd agentbus && go test -run TestQuizStartCauses ./...`; a single bats test:
  `bats -f "watchdog" tests/unit`.
- After changing `agentbus/errors.json`: `make generate`.
- All checks must pass with **zero** findings. Fix the cause; do not add `//nolint` or `# shellcheck disable`
  without a comment on the same line explaining why the finding does not apply, and mention every such
  suppression in the PR description.

Do not push code that fails `make lint test`. If a check fails for a reason outside your change (a broken tool,
a flaky test), do not work around it: describe it in your report and ask.

## Go

### Errors — the model

Every operation returns its **own error type**, so the signature shows what can fail:

```go
func (s *Server) quizStart(question string) (string, *QuizStartError)
```

- `QuizStartError` is `OpError[QuizStartCause]`; `QuizStartCause` is an enum of exactly the failures of
  `quiz_start`, generated from `agentbus/errors.json`. Each cause has a class (`InvalidArgument`, `NotFound`,
  `NotInTeam`, `Permission`, `Conflict`, `InvalidState`, `Storage`, `UnknownTool`, `Process`) that gives the
  sentinel for `errors.Is` and the exit status of CLI subcommands. A `Storage` cause is added automatically.
- Lower layers return concrete types: `*StorageError` (files, stdout), `*ValueError` (invalid values),
  `*ProcessError` (helper processes). An operation converts them explicitly: `fromStorage(QuizStartStorage, serr)`,
  `fromValue(BoardAddInvalidLevel, verr)`.
- State changes go through `transact(bus, <Op>Storage, func(st *State) *<Op>Error {...})`, which turns lock,
  load and save failures into the operation's storage cause.
- Internal helpers that check something return a result enum, not an error (see `requireLead` → `leadCheck`);
  each operation maps the result to its own cause.
- The plain `error` type is allowed **only** where an interface requires it (`json.Unmarshaler`,
  `slog.Handler`, `flag.Value`) and in the MCP dispatcher. Convert with `asError()` (never assign a typed nil
  pointer to an `error`) and say in the comment which concrete type is inside.
- **Never ignore an error.** No `_ = f()`, no unchecked `Close`. Either the error fails the operation, or it is
  a side effect that is logged (`warnSideEffect(effect, err, attrs...)`; `warnOnce` for pollers). Decide which
  and say why in a comment when it is not obvious.
- Messages are for agents and humans: what went wrong and, where possible, what to do instead.
- "Not an error" results stay results: `TAKEN`, `QUEUE FULL` and similar answers are returned as output, not
  as errors.

### Types

- Use the type system to make invalid states impossible. Statuses, states, tiers, levels, kinds, roles and log
  event names are **typed constants** (`agentbus/types.go`, `events.go`, `logging.go`), never string literals
  in logic. A new enum gets `Valid()`, is parsed with `parseEnum`, and rejects unknown values in
  `UnmarshalJSON` (with a `*ValueError`).
- Times are `Timestamp`; its zero value means "has not happened yet".
- Every bus message is an event type implementing `busEvent` (`Kind()`, `Text()`) in `agentbus/events.go`;
  post it with `bus.post(st, from, to, event)`. Do not build message texts at the call site.
- Prefer small structs with named fields to multiple return values of the same type.

### Comments and naming

- **Every constant, variable, type and struct field has a comment** that tells a reader what it is for
  (`doccheck` enforces it; generated code gets its comments from `errors.json`). Exported identifiers: the
  comment starts with the name.
- Comments explain *why* and the contract (units, ownership, what nil/zero means, what can fail), not what the
  next line obviously does.
- English everywhere: identifiers, comments, messages, logs, tests, commit messages.

### Logging

- `log/slog` through `logInfo` / `logWarn` / `logError` with a `LogEvent` constant (snake_case, stable: dashboards
  and alerts use the names). New events are added to the `LogEvent` list with a comment.
- Records share one schema with the script and the panel: `ts`, `level`, `component`, `event` plus fields. No
  `fmt.Print*` for diagnostics; stdout of subcommands is a parsed interface (use `cliWriter` / `printJSON`).

### Style

- `gofmt`; small functions with one job; early returns; no global mutable state beyond what exists (and when
  you must, document who owns it).
- Concurrency in `agentbus` is between processes: all shared state is read and written inside `transact` /
  `locked`. Never wait or sleep while holding the lock; poll outside it (see `reserve`, `waitForMessage`).
- Keep `agentbus` free of third-party dependencies.

### Adding an MCP tool (checklist)

1. Add the operation and its causes to `agentbus/errors.json`; `make generate`.
2. Declare the tool and its argument types in the tool list (`toolSchemas` / `*ToolDefs`) and add the handler
   returning `(string, *<Op>Error)`; dispatch it in `callTool` with `toolResult(...)`.
3. Bus events for it in `events.go`; log events in `logging.go` if needed.
4. Tests: every cause (`failsWith(t, <Op><Cause>)(...)`), the success path, and the dispatcher.
5. Document it in the README ("MCP tools available to agents") and, if agents should use it, in the prompts.

## Bash (`bin/agent`, scripts)

- `#!/usr/bin/env bash` with `set -euo pipefail`; `shellcheck -S warning` clean with the pinned version.
- Quote every expansion; use arrays for argument lists; `local` for every function variable; a one-line comment
  above every function saying what it does and its arguments.
- **No silent errors.** No bare `|| true`, `2>/dev/null` or `|| :`. Use `try <event> cmd ...` (runs a command,
  logs a failure as a warning, continues) or `jq_soft` for JSON that may be malformed; when a non-zero exit is
  an expected answer (e.g. `grep` finding nothing, `agentbus` exit code `BUS_NOT_FOUND`), handle that code
  explicitly and comment it.
- Log with `log_info` / `log_warn` / `log_error <event> key=value...` (snake_case event names). Publish events
  for agents on the bus with `notice <kind> ...` (typed script notices), not free text.
- Configuration variables get a default with `${VAR:-default}` and a comment, and appear in
  `config/agents.conf` and the README.

## Prompts (`prompts/*.md`)

- English. Placeholders `{{NAME}}` must be set in `build_prompt` (`R[...]`); a placeholder that is not set stays
  in the prompt — a bats test checks that rendered prompts contain no `{{`.
- Keep the protocol consistent across `protocol.md`, `team.md`, `investigator.md`, `helper.md` and `review.md`;
  when you change a rule in one, check the others.
- Prompts are instructions for other models: short, unambiguous, imperative; say what to do when something
  fails.

## Tests

- **New code comes with new tests**; a bug fix comes with a test that fails without the fix. Remember rule 1:
  existing tests are not yours to change.
- Go: check errors by their exact cause (`failsWith(t, BoardClaimAlreadyTaken)(...)`, `failure(...)` when you need
  the error, `expectToolError` for dispatcher errors) or with `errors.Is` / `errors.As` — never by comparing
  message texts (the helper `missing(...)` is the one message that is part of a contract). Use `newBus(t)`
  (temporary state directory), `server(...)`, `createTeam(...)`, `captureLogs(t)`.
- bats: every test runs in a temporary `$HOME` with stub CLIs (`tests/unit/stubs/`). Drive the fake agent with
  `scenario '{"<agent>": {"new": "<action>"}}'`; wait with `wait_status` / `wait_grep`, never with a bare
  `sleep`; assert on files, `jq` queries over logs and git, not on timing.
- Tests are deterministic, need no network and no real accounts, and clean up after themselves.

## Documentation

- Every user-visible change updates the README in the same PR: commands, configuration, files, MCP tools, log
  events, exit codes, security implications.
- `config/agents.conf` and `config/secrets.env.example` are templates: keep them in sync with the code.
- Write for someone who installs team-agents for the first time.

## Commits and pull requests

- The orchestrator sets the commit author and gives you the trailers to add; do not change git identity.
- Small commits, each building and passing the checks. Subject in the imperative mood, at most 72 characters
  (`Add board_reassign tool`, `Fix lost wake-up after a crash`); the body says why, and what the reader cannot
  see in the diff.
- The PR description: what and why, how it was tested (commands and results), compatibility notes (rule 9),
  every lint suppression, and anything the reviewer should look at first.

## Definition of done

- [ ] `make lint test` passes with zero findings.
- [ ] New behaviour has tests; no existing test was changed.
- [ ] Errors are typed per operation; nothing is ignored; enums instead of string literals; every declaration
      has a comment.
- [ ] README, `config/agents.conf` and prompts updated where the change is visible to users or agents.
- [ ] Compatibility of on-disk formats, subcommand output and exit codes kept (or migrated, tested and
      documented).
- [ ] The report and the PR description explain what was done, why, and what is left.
