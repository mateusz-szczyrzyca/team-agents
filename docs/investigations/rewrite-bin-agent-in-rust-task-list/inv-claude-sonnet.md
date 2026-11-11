# Rewrite `bin/agent` in Rust — task list: review of the plan

Independent Phase 1 result of `inv-claude-sonnet` (task `20261111-141417-inv-claude-sonnet`), based on
[`27b39ba`](https://github.com/mateusz-szczyrzyca/team-agents/commit/27b39ba293e36633a7c07ec562d0d3c9b8626725).
No code was changed; this file is the whole result.

**What I did.** I read the task list as a plan to be executed by the harness that exists today (`bin/agent`, Bash) and
checked every item against the repository as it is: the harness's own rules and size limits, `agentbus/` (which "stays
and is used as it is"), the old CLI surface, the CI and gate wiring, and the external APIs the plan relies on. Where a
claim could be tested cheaply I tested it (toolchain, build time, `reqwest`, GitHub API). Statements are marked
**verified** (I read it or ran it), **external** (documentation or search result) or **estimate** (my judgement).

## 1. Verdict

The plan is coherent and the target architecture (one daemon owning the state, typed core, effects in a runtime crate,
`agentbus` as a process) is sound. It cannot be executed as written, for five reasons, in order of how soon they bite:

1. The two rule files every item is told to follow **do not exist** in the repository (section 3, B-1).
2. The first item changes files that the running harness forbids or holds for the human, and the harness's size and
   test checks do not understand Rust (B-2 … B-4). Without preparation **every** Rust PR is bounced by the precheck.
3. `agentbus` calls back into the `agent` CLI through hidden subcommands (`_wake`, `_dispatch-helpers`,
   `_board_done_check`). The plan says nothing is kept compatible, so as written these hooks would fail silently (H-1).
4. Three ordering errors and at least two items that will exceed the 5000-line limit (H-4, H-5).
5. The cutover (`10.1`, `10.2`) cannot be done by the agents themselves, is not rehearsed anywhere, and silently removes
   capabilities the plan lists as "out of scope" (H-6, H-7).

All of it is fixable with edits to the plan and a handful of human actions before `1.1` starts (sections 5, 8 and 9).

## 2. Evidence base and what I could not check

| Checked | Result |
|---|---|
| `prompts/agent/rust-developer.md`, `prompts/investigator/code-review.md`, `prompts/worker-agent/…`, `prompts/investigator/rust-developer.md` | **missing** at the base commit and on every fetched remote ref (`git grep` over `refs/remotes`, `git log --all -- prompts/agent` are empty). `prompts/` is flat. |
| Rule IDs (`ERR-2`, `PR-1`, `PR-2`, `PR-6`, `TYP-1`, `SEC-2`, `SEC-3`, `SEC-5`, `CON-3`, `CON-4`, `CON-5`, `DEP-1`) | **cannot be verified**. I judged the items against the plan's own text only. |
| Toolchain on this host | `cargo 1.99.0`, `rustc 1.99.0`, `clippy 0.1.99`, `rustfmt 1.10.0`, `crates.io` reachable (**verified**) |
| Cost of the gate's Rust part | scratch crate with all 16 approved dependencies (388 packages): cold `cargo build` **13.5 s**, `clippy -D warnings` **4.9 s**, `cargo doc` and `test --no-run` under 1 s, on 32 cores (**verified**). The 10-minute `TEST_LOCK_STALE_MIN` in [`tools/test-lock.sh`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/tools/test-lock.sh#L24) is **not** at risk here; a 2-core CI runner is slower (not measured). |
| `time` crate local offset | `OffsetDateTime::now_local()` worked from a multi-threaded process with `time 0.3.55` (**verified**). The risk I suspected did not materialise. |
| `reqwest` and Unix sockets | `ClientBuilder::unix_socket` exists since `0.12.23` (**external**, [release notes](https://newreleases.io/project/cargo/reqwest/release/0.12.23)): the Tailscale LocalAPI needs no `tailscale` process and no extra dependency. |
| Real model CLIs, real GitHub | not run (rule 4 of `AGENTS.md`); GitHub behaviour is from documentation. |

## 3. Findings

Severity: **Blocker** = the first item cannot start or pass; **High** = an item will fail or silently misbehave;
**Medium** = a decision or a gap to settle before the item starts; **Low** = polish.

### Blockers (before `1.1`)

#### B-1 The rule files do not exist

Ground rules: "Read `prompts/agent/rust-developer.md` … Every PR is reviewed against `prompts/investigator/code-review.md`.
Both are binding". `1.1` takes "the lint baseline from the appendix of the developer rules". `2.3` says the rules are
"provided by the human". At the base commit none of them is in the repository, so `1.1` has no lint baseline, no rule
IDs resolve, and reviewers have nothing to review against.

A second consequence: the current harness renders its reviewers from
[`prompts/review.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/prompts/review.md)
(`bin/agent` `build_prompt`, lines 4175-4225) and never mentions `prompts/investigator/code-review.md`. Until cutover the
only way reviewers meet the new review rules is through `AGENTS.md` (which the review prompt includes as `{{AGENTS_MD}}`).
So the Rust section of `AGENTS.md` that `1.1` adds must name both files explicitly, with "reviewers apply
`prompts/investigator/code-review.md`".

**Fix:** the human commits the files to `main` before `1.1` (a human step, section 8). I could not review the plan
against rules I cannot read; the rule-based part of this review is therefore missing.

#### B-2 `1.1` edits files the harness forbids or holds

`1.1` must change `AGENTS.md` and "the repository's existing gate and CI". In the harness that runs this plan:

- `AGENTS.md` and `CLAUDE.md`: Never rule 2; the precheck (`round_rule_issues`,
  [`bin/agent#L2051`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L2051))
  sends the round back for **any** change unless the human ran `agent allow <team> AGENTS.md`
  ([`cmd_allow`, L8082](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L8082)).
- `.github/workflows/test.yml`: Never rule 6; same precheck rule. The workflow lists its steps explicitly
  (`make lint`, `make unit-go`, `make unit-bats`) and sets up Go only.
- Autopilot never merges a PR that touches one of
  [`AUTO_MERGE_HUMAN_PATHS="install.sh .github/ systemd/ AGENTS.md"`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L698).
  `systemd/` is matched as a directory name anywhere in the path (`rule_path_matches`), so `config/systemd/` is held too.
  The PRs handed to the human regardless of the reviews: `1.1` (`AGENTS.md`) and `10.1` (`install.sh`) for certain, `6.1` and `9.1` if their systemd units live under a `systemd/` directory (`config/systemd/` in the target layout).
- `tools/gate.sh` hard-codes its steps
  ([`STEPS=(…)`, L16](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/tools/gate.sh#L16));
  `unit-agent-bats` is added only `if` the Makefile has the target. A new Rust step must use the same conditional
  pattern, because `tests/harness/gate.bats` (an existing test) runs `gate.sh` against fake Makefiles.

**Fix (choose one for CI):**
- (a) put the Rust checks inside the existing `lint` and `unit-go` Makefile targets, so CI runs them with no workflow
  edit. `ubuntu-latest` has `rustup`; a `rust-toolchain.toml` with `components = ["clippy", "rustfmt"]` makes it install
  the pinned toolchain. This works only if CI is checked after the first push.
- (b) the human edits `.github/workflows/test.yml` (needs `actions/checkout`, a Rust step, no secrets).
Either way: `agent allow <team> AGENTS.md` before the round starts, and expect to merge `1.1` by hand.

#### B-3 The harness's size limit contradicts the relaxed limit

The plan relaxes PR-1 to 5000 lines, tests not counted. The harness enforces
[`ROUND_MAX_LINES=3000`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L703),
tests included, excluding only
[`*_gen.go go.sum package-lock.json vendor/`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L704).
Consequences, all **verified** in the code:

- `Cargo.lock` is counted (it is not `go.sum`); the first PR with dependencies carries thousands of lines of it.
- Rust tests are counted. They can be excluded by path only if they live in files a pattern can name (`tests/`,
  `tests.rs`); inline `#[cfg(test)] mod tests { … }` blocks cannot be excluded at all.
- A round over the hard limit goes back to be split, `ROUND_BOUNCES=2` times, then to the human.

**Fix:** the human sets, in `~/agents/agents.conf` (agents may not edit it), `ROUND_EXCLUDE` to add
`Cargo.lock tests/ tests.rs fixtures/` and `ROUND_MAX_LINES_REPO[team-agents]=5000`. In return the plan fixes one
convention in `AGENTS.md`: **tests live in `tests/` directories or sibling `tests.rs`/`tests/mod.rs` files, never inline**.
(The convention is also what B-4 needs.)

#### B-4 The precheck does not recognise Rust tests

Before every review the harness requires "the round changes code ⇒ it adds tests"
([`review_precheck`, L2673](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L2673),
[`round_test_changes`, L1672](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L1672)).
Tests are found with `TEST_PATHSPECS`, whose only Rust entry is
[`**/tests/**/*.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L1669),
and counted by the lines `#[test]`, `@test`, `func Test` … A PR whose tests are inline (`src/foo.rs`) or in `src/foo/tests.rs`
adds **zero** tests to the harness and is bounced with "the round changes code but adds no test"
(`AGENT_PRECHECK_MAX=2`, then the human decides). The same file gives the rules B1/B3 for existing tests and
test-aware production code only for `tests/*` (repository root), `*_test.go`, `*.bats` and `bin/ agent/ agentbus/ live/`:
**nothing under `crates/` is protected by Never rules 1 and the "runs under test" rule**, so these rules would rest on
reviewers alone. (B2 does cover `#[allow(...)]` at the start of a line, but not `#[cfg_attr(…, allow(…))]`.)

**Fix, either:** (a) the convention of B-3 — Rust tests only in `crates/*/tests/**/*.rs` (integration tests of the public
API; `agent-core`'s pure types are public anyway) and the round-size patterns; or (b) one small PR to `bin/agent` that
adds `:(glob)**/tests.rs` and `crates/**` to the pathspecs and the `rule_*_path` functions (with its own bats test). (a) is
cheaper and needs no harness change; (b) is more honest for `#[cfg(test)]` unit tests. Pick one and write it into the
`1.1` text; otherwise the first four Rust PRs are bounced for a reason the authors cannot see in the plan.

### High

#### H-1 `agentbus` calls back into the `agent` CLI

The plan: "Nothing is ported line by line, linked, called or kept compatible: no shared commands" and "`agentbus/` stays and
is used as it is". But `agentbus` is not passive; it executes the `agent` command it was given (`--agent-cmd`, and a
`wake_cmd` stored in every bus team):

| Callback | Where | Contract |
|---|---|---|
| `agent _board_done_check <task>` | [`agentbus/collab.go#L617`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/collab.go#L617), [`L102-L130`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/collab.go#L102) | called inside the MCP tool `board_done`; `AGENTS_HOME` in the environment; exit `0` allow, **exit `3` refuse** (stdout is the reason shown to the agent), anything else or 60 s timeout = "no answer": the result is let through and logged |
| `agent _dispatch-helpers` | [`collab.go#L934`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/collab.go#L934) | detached, after every `delegate` request |
| `agent _wake <task>` | [`agentbus/team.go#L96`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/team.go#L96) | detached, for every message to a sleeping member |

If the new binary lacks these, nothing crashes: the failures are only logged. Result: the board-done gate check stops
being enforced (workers can submit unverified work), `delegate` requests are queued but never served (`7.1` "delegate
requests served by the first free worker"), sleeping members are never woken by messages (`7.1` "wake-ups on bus
messages"). And `5.1`'s MCP configuration must pass `--agent-cmd` and `--home` to `agentbus serve` (see
[`bus_args_json`, L4386](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L4386)
for the full flag list: `--agent --task --task-dir --repo --branch --worktree --peers --role --queue-max --inv-slots
--tasks-dir --team --tier --agent-cmd --home`).

**Fix:** add to `6.1` (the daemon's first client commands): the three hidden subcommands as thin clients of the daemon
socket, with the exit-code contract above; the daemon passes its own binary as `--agent-cmd` and as the team's wake
command (`4.2`); a test with the real `agentbus` built from `agentbus/` that makes it call each of them. The daemon must
also scan the bus for messages that arrived while it was down (recovery in `6.1`), since the callback fails with no
daemon. Alternative (watch `bus/messages.jsonl` with `notify`) would duplicate wakes unless the callback is disabled,
and `agentbus` cannot be changed.

#### H-2 Two stores for teams, members and the board

`2.2` puts "teams, members and their roles, the task list with per-item progress" into the daemon's state. `agentbus`
already persists teams, member states (`active|sleeping|waking|done|excluded|hung|paused`), the lead, the board (`B1…`),
delegation queue, consults and a quiz in `bus/state.json` (`Team` in `agentbus/team.go`). `4.2` wraps `team-create/set/done`,
`board-add/claim/dump`, `helper-*`, `checkpoint-get`. Nothing says which store is authoritative, so two writers will
disagree after the first crash.

**Fix (record in `docs/state.md`, task `2.2`):** the bus is authoritative for what agents mutate through MCP (messages,
board items and claims, delegation queue, notebook, checkpoints); the daemon is authoritative for tasks, runs, outcomes,
PRs, reviews, gate results and for member *availability* (it pushes member state to the bus with `team-set`); on start the
daemon rebuilds its projection of the bus, never the reverse. Also state how a task-list item `2.3` maps to board items
`B7` (the coordinator creates board items for workers with `board_add`; the daemon assigns task-list items to members —
two different things with similar names). Note that `board-add`, `board-claim`, `claim-lead`, `delegate` exist as CLI
subcommands but are missing from `agentbus`'s usage text
([`main.go#L1454`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/agentbus/main.go#L1454)):
undocumented, so `4.2`'s tests against the real binary are their only specification.

#### H-3 The GitHub part has gaps that fail in practice

`4.2` / `8.1` need these corrections; each would otherwise surface as a failed merge loop:

- **"mark ready" is not in the REST API.** `4.2` lists it among REST calls; REST cannot clear `draft` — only the GraphQL
  mutation `markPullRequestReadyForReview` does ([**external**](https://docs.github.com/en/graphql/reference/mutations#markpullrequestreadyforreview)).
  The old harness uses `gh pr ready` ([L2392](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L2392)).
  Same `reqwest` client, one GraphQL POST; needs the PR's node ID. Alternative: open the PR non-draft once the gate is
  green (the plan already opens it only then) and skip "mark ready" — simpler.
- **Reviews are by the PR's own author.** All agents act through one token, so the reviewers' account is the PR's author.
  GitHub refuses approving one's own PR (**external**), and I expect it to refuse requesting changes too (unverified — check against a real PR); reviews must be
  posted as `COMMENT` reviews or issue comments with the verdict in the text. `4.2` should expose only that, and
  attribute each review to its agent and model in a byline (the old harness has `gh_attribution`, per-agent colours,
  `review_byline.bats`; the plan has nothing, so a human cannot tell the two reviews apart).
- **Merge only the reviewed commit.** The old merge passes `--match-head-commit` with the checked head
  ([L2399](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L2399)):
  anything pushed after the reviews stops the merge. REST: `PUT /pulls/{n}/merge` with `sha` and `merge_method=merge`
  (the repository history uses merge commits). Put it in `8.1` as a requirement and a test.
- **CI and branch protection.** `8.1` merges after two reviews; the repository's CI runs on every PR. The plan does not
  say whether a red or pending CI blocks the merge. If branch protection exists, the merge call returns `405`; that must
  be "wait / ask the human", not an error that stops the daemon.
- **Conflicts and moved base.** Features run in parallel, so open PRs of one team can conflict after a sibling merged.
  `8.1` says nothing on what happens at a merge conflict (back to the coordinator to merge the base, re-gate, and
  whether the two reviews stay valid). The old harness also runs the gate on the merged base after the merge
  ([`gate_at origin/$base`, L2410](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L2410)) and stops the autopilot if it fails; the plan starts the next item on an unchecked base.
- **Idempotence.** "open the PR" must first look for an existing PR from that head branch (crash between create and
  state save otherwise creates a duplicate, or fails on "already exists"); "merge" must treat "already merged" as done.
  Persist the intent before the call, reconcile after a restart.
- **Who opens the PR?** "Behaviour in short" says the coordinator "opens the PR"; `8.1` says the daemon does. Pick the
  daemon (consistent with "one owner") and fix the sentence; and say how agents are kept away from `gh`/the token (the
  "agents' PATH" of `2.1` can leave `gh` out; the old harness shipped a filtering `gh` wrapper for exactly this and for
  prompt-injection from untrusted comments — `4.2`'s "filtered to the trusted accounts" covers only the daemon's reads).

#### H-4 Ordering defects inside the dependency graph

Checked against the plan's own rule "an item uses only code from the features named in its Depends-on line":

- `2.2` (questions files) says "the instruction text comes from a template"; the template catalog and renderer are `2.3`,
  taken **after** `2.2`. Reorder (`2.3` first) or have `2.2` take the text as a parameter.
- `7.1` (team life: "a member done → the coordinator integrates … gets the gate green") needs the gate service, which
  is `7.3`, taken last. The prompts of `7.1` would tell agents to run `agent gate`, which does not exist yet, and
  `_board_done_check` (H-1) is also gate-based. Take the gate service first (`7.3` → `7.1`).
- `9.1` depends on `6` only, but shows PRs (`8`), the answer form and controls (`7.2`) and the task-list progress (`7.1`).
  With only `6` merged the buttons call handlers that do not exist. Split: `9.1` read-only view and SSE on `6`;
  `9.2` controls and the answer form on `7.2`.
- `4` depends on `2`, but `4.2` (HTTP client, `agentbus` wrapper) needs only `1` and the `secrets.toml` type; fine as is,
  just not on the critical path — start it after `1.1`+`2.1` if parallelism is wanted.

Critical path as written: `1.1, 2.1, 2.2, 2.3, 4.1, 4.2, 6.1, 7.1, 7.2, 7.3, 8.1, 10.1, 10.2` = **13 serial PRs**, each
with two reviews and up to five fix passes. Features `2.x` are largely independent (`2.1` config/task list, `2.3`
prompts/CLI contract, `2.2` state/protocol after `2.3`) and `7.2`/`7.3` do not depend on each other, so those can be
separate features and run in parallel. The width the graph allows is 3 at most (after `2`: `3`, `4`, `5`), and 1–2 for
most of the rest, so a team of more than three normal members is idle most of the time.

#### H-5 Item sizes

The plan's own rule: an item that would exceed 5000 lines is split **before coding**. My **estimates** (non-test lines,
Rust at about twice the lines of the equivalent Bash because of types, docs under `missing_docs`, error enums). The Bash
reference sizes come from the top-level function sizes of `bin/agent` (11,641 lines, 405 top-level functions):
runner, watchdog, limits and resume about 1,900 lines; PRs, reviews, merge, rounds and rules about 1,400; teams, board,
helpers about 700; questions and notifications about 400; gate about 200; prompt building about 300.

| Item | Estimate | Risk | Why |
|---|---|---|---|
| `1.1` | 2.5-3.5k | medium | workspace, gate, docs, `AGENTS.md`, seven ID types, four enums, state machines; mixes tooling that the human must merge by hand with code that could auto-merge — split into `1.1` (workspace, gate, docs, `AGENTS.md`) and `1.2` (vocabulary) |
| `2.1` | 1.5-2.5k | low | |
| `2.2` | 2-3k | low | |
| `2.3` | 1-2k plus prompt files | low | |
| `3.1` | 2.5-3.5k | medium | formation, assignment, round machine and outcome classification in one item; the reviewer-selection function is missing here (M-1) |
| `4.1` | 3.5-5k | **high** | store, flock, activity log, prompt loader, supervisor, a dozen git operations, notifications. Split into store+loader+notifications and supervisor+git |
| `4.2` | 2-3k | low | |
| `5.1` | 3-4.5k | medium-high | four stream parsers with fixtures; split `claude`+`codex`, `agy`+`opencode` |
| `6.1` | **5.5-7.5k** | **over the limit** | daemon, socket, 12 request types, shutdown, recovery, `status`, systemd unit, the whole run engine, watchdog with strikes, quota/rate-limit pauses and timers, plus H-1's compat subcommands. Split now: daemon+socket+recovery+status, run engine, watchdog+limits |
| `7.1` | 3-4.5k | high | `run` command, agentbus team, worktrees, assignments, integration trigger, delegation, wakes, closing |
| `7.2` | 1.5-2.5k | low | |
| `7.3` | ~1k | low | |
| `8.1` | **4-5.5k** | **at/over the limit** | PR open/update, reviewer selection and runs, verdict parsing, review posting, merge, fix passes, conflicts, CI wait, post-merge gate. Split: open/update PR; reviews; merge loop and fix passes |
| `9.1` | 3-5k | high | axum server, LocalAPI auth, templates, SSE, forms, controls, unit; split as in H-4 |
| `10.1` | ~2k, mostly README | low | |
| `10.2` | see H-6 | — | |

Calibration: the unfinished Go port in `agent/` (the plan's "unfinished Go") holds 6.5k production lines (config, meta,
the `gh` filter, IDs, `show`, `status`) after 400 commits, and not yet a runner — ambition of this plan is far higher, which
is another reason to split early. The test kit gap (M-5) adds to every item from `6` on.

#### H-6 Cutover: not rehearsed, not doable by the agents, not mechanical for the harness

- **No end-to-end rehearsal.** Items `6`–`8` are tested against fakes, and rule 4 forbids real CLIs. Nothing in `1`-`10`
  runs the whole loop (`agent run` → work → gate → PR → two reviews → merge) against stub CLIs, a fake GitHub and a real
  `agentbus` in a scratch `AGENTS_HOME`. Add `10.0`: that rehearsal as an automated test plus one documented manual
  dry run, before `10.1`.
- **Fixtures of "real CLI outputs"** (`3.1`, `5.1`) cannot be recorded by the agents (rule 4); the existing stubs
  (`tests/unit/stubs/fake-claude` etc.) are synthetic, and existing stub behaviour cannot be changed. Sources: sanitized
  logs the human provides from `~/agents/tasks/*/out-N.jsonl` (reading those is not an edit, but they contain real
  hostnames and may hold secrets — SEC-2), or a one-off, human-authorised recording. Quota-exhausted output cannot be
  produced on demand at all; the human has to supply real samples. Question for the human (Q4).
- **`10.1` cannot be run by agents**: `install.sh` against the live home is forbidden (Never 3), and the plan's own
  "nothing may touch the live `~/agents` until feature 10" — an agent runs inside it. `10.1` must stop at "the files",
  and the switch itself (stop the old harness, install, start `agent daemon`) is a human step with a written
  procedure and rollback. Also decide the **new default `AGENTS_HOME`**: the new state format shares nothing with the old,
  but `~/agents/` already holds `bus/`, `tasks/`, `worktrees/`, `logs/` in the old layout (and `agentbus`'s `bus/state.json`
  must not be mixed with a running old harness). Recommendation: a new directory, with a one-way import of nothing.
- **`10.2` is not "mechanical" for the harness.** It removes about **66,000 lines** in about 455 files (`bin/agent`
  11.6k, `agent/` + `live/` 244 files, `tests/` 211 files, 22k lines) — over the 3000-line round limit by a factor of 20;
  deletes tests, which the precheck sends back unless each exact path is allowed (`agent allow` takes exact paths,
  no globs or directories); changes `Makefile` (`GO_MODULES`, `unit-go`, `lint-docs`, `check-generated`,
  [L8](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/Makefile#L8)…),
  `tools/gate.sh` and CI (forbidden); and must not forget `tools/check-agent-*.sh`, `tests/agent-go-map.tsv`,
  `tests/harness/*` that reference the old script, `prompts/*.md` (the old flat prompts, which `10.2`'s list omits),
  `config/agents.conf`, `examples/notify/`, `docs/agent-*.md`, `rewrite-agent-in-go.md`. **Recommendation:** `10.2` is a
  human action (`git rm` after a soak period of the new daemon running the live harness), and the plan only lists the
  files; or the human prepares `agent allow` with `git ls-files bin agent live tests | xargs agent allow <team>` and sets
  `ROUND_MAX_LINES_REPO[team-agents]=0` for that team.

#### H-7 Capabilities the cutover removes, and CLI surface the plan never mentions

"Out of scope" lists remote nodes and the lead, investigation teams, `agent live`, archiving, token and cost accounting,
the precheck rules, automatic PR size limits. `10.2` then deletes the code for all of them. That is a decision
(this very task runs as an investigation team, a feature of the old harness), not a detail; state it in `10.1`'s
documentation and make the human confirm it (Q6). Safer: keep `bin/agent` until the features the human still uses are
ported, and make `10.2` conditional on that list.

Beyond that, the old CLI ([`usage`, `bin/agent` L11500-L11580](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L11500))
has commands the plan neither provides nor excludes:

| Old command | Plan | My recommendation |
|---|---|---|
| `run`, `status`, `show`, `answer`, `stop`, `pause`, `unpause`, `say`, `gate`, `web` | covered | — |
| `resume` | only "the task resumes with the answers"; `run --resume <team\|task> [--same-round]` (continue an unmerged round after review fixes, resume a stopped/hung team) is not covered | add to `7.2`: resume of a stopped, failed or hung task or team |
| `team <id> --stop/--close`, `stop <team>` | not mentioned | needed: stopping a team of N members one by one is unusable |
| `autopilot <team> on|off` | `--no-merge` at start only | add switching at run time (cheap: a flag in state) |
| `kick`, `wake` | `wake`'s role (end a quota pause now) not mentioned | add `wake`; `kick` optional |
| `agents` (who is configured and busy), `log <id>`, `transcript`, `bus` | partly "state and activity show what was done" | add `agents` and `log`; they are the first things used when something is wrong |
| `authenticated` (probe each agent's login and model) | no | add: login and quota problems are the most common operational failure |
| `clean`, `discard`, `ack` | worktree removal is a typed git operation in `4.1`, but **nothing calls it**: worktrees of merged items pile up | add automatic cleanup after merge/close to `7.1`/`8.1` |
| `notes` (repository notebook), `helpers`, `round-check`, `round-size`, `allow`, `gate-status`, `debug` | no | `notes`/`helpers` are thin `agentbus` wrappers; `allow`/`round-*` belong to the dropped precheck; `debug` is useful but can wait |
| `tokens`, `spend`, `archive`, `live`, `lead`, `node`, `jobs`, `nodes`, `inv`, `dispatch` | out of scope | as stated; note the loss in `10.1` |
| `web --devices` (tagged tailnet devices) | "allowed logins" only | regression: the old web admits tagged devices through `whois` ([`live/web.go#L47`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/live/web.go#L47)); decide (Q9) |

`docs/agent-inventory.md` (382 lines) is a ready checklist of commands, configuration variables, persisted files and
locks of `bin/agent`; the plan should cite it as the behaviour reference instead of "`bin/agent`" in general.

### Medium

#### M-1 Reviewer selection is under-specified, and sits in the wrong item

"Two free investigators from outside the team (the PR waits until two are free)" is effect-level text in `8.1`, but it is
a pure decision that belongs in `3.1` with tests. It must define:

- **free** = no active model run. A coordinator waiting for reviews has no run, so it counts as free for other teams;
  otherwise two teams led by investigators can wait for each other forever. Add this as a named deadlock test.
- an investigator paused by a quota is **not free** (the old harness excludes it, `_investigator_is_busy`).
- with `inv-1..inv-4` and two teams each led by an investigator, who reviews what, and how is a pair chosen (random,
  rotation — the old harness has `review_pair_random.bats`, `review_pair_rotate.bats`, `review_pair_models.bats`).
- a solo investigator task needs two **other** investigators too; the "too many investigators in the team" refusal must
  cover it (a team of one).
- the verdict set: the old harness distinguishes `ready-to-merge`, `needs-fixes`, `incomplete` and no verdict (and lets
  critical/major findings outweigh a `ready-to-merge`;
  [`review_verdict` L1653](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L1653)).
  The plan has "changes requested" and "ready"; a review that ends with no verdict (crash, hang, quota) needs a defined
  outcome: re-run by a third investigator, or ask the human after `AGENT_REVIEW_WAIT_MIN`-style waiting.
- a human-visible state and a notification when a PR waits longer than a configured time for two free investigators.

#### M-2 A daemon restart kills every run

The old harness ran each task in its own transient systemd unit, so a restart of the harness left runs alone. In the new
model every model CLI is a child of the one service; with the default `KillMode=control-group`, `systemctl --user restart`
(an upgrade, a crash) kills all runs, and `6.1` then resumes each from its session — one prompt and its tokens per run,
on every upgrade. Options: accept it and say so; or `KillMode=process` and let the daemon re-adopt live children on start
(`4.1` already sends stdout and stderr to files and uses a process group per run, so adoption by recorded PID is possible).
Decide in `6.1`, and add the choice to `docs/architecture.md`.

#### M-3 The socket has no caller identity

Mode `0600` authenticates the OS user, and every agent runs as that user with `--dangerously-skip-permissions` /
`--dangerously-bypass-approvals-and-sandbox` (see
[`bin/agent` L4568, L4624, L4664](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L4568)).
An agent can therefore call `answer`, `stop`, `resume` and `say` on the socket as the human. Agents legitimately need
only `gate` and the three hidden callbacks. Give each run a token in its environment and let the protocol restrict a
token to those requests (requests without a token = the human). The plan also never mentions the permission/sandbox
flags the adapters must pass; `5.1` should state them (and that the GitHub token is never put into a run's environment).

#### M-4 Dependencies

- **SSE needs a `Stream`**: `axum`'s `Sse` takes a `futures_core::Stream`; making one from the daemon's subscription needs
  `futures-core` and normally `tokio-stream` — neither is approved. Expect a DEP-1 for `9.x` (or hand-write the stream
  with `tokio_util::sync::ReusableBoxFuture`, which is more code than the dependency).
- **Time zones**: provider messages like "resets 3pm (Europe/Warsaw)" are read today with `date -d` (local zone; the
  parenthesised zone is ignored; [`quota_reset_seconds`, L3866](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L3866)).
  The approved `time` crate has no tz database; ISO times and durations are fine, named zones are not. Keep the old
  behaviour (local zone) and say so, or ask for `jiff`.
- **Pattern matching of provider messages** (`QUOTA_PATTERN`, [L3790](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L3790))
  is a regex today; `regex` is not approved. The patterns are plain alternations of phrases, so substring matching works;
  do not let a reviewer push a `regex` dependency in through `3.1` without a DEP-1.

#### M-5 No shared test kit

Items `6`–`8` need fake model CLIs for four output formats, scripted scenarios (finish, ask questions, hit quota, hang,
fail), a temporary git remote, a fake GitHub (`4.2` builds one — reuse it) and the real `agentbus`. The existing stubs
are Bash/Python, belong to the old harness and may not be changed. The layout has four crates and no place for this.
Add `crates/agent-testkit` (`publish = false`, dev-dependency only) in `1.1`/`1.2`, or at the latest `4.1`, with the
fixtures of `5.1` as its data. Also: the Rust integration tests need `agentbus` built first — one Makefile dependency.

#### M-6 Prompts

- `prompts/clarification.md` is listed in the target layout as new, but it exists and the old harness renders it
  ([`bin/agent#L4254`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/bin/agent#L4254));
  changing it changes the running system before cutover. Use a new name (`prompts/harness/clarification.md`) or leave
  it untouched.
- `install.sh` copies only [`prompts/*.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/27b39ba293e36633a7c07ec562d0d3c9b8626725/install.sh#L137)
  and the bats helpers copy `prompts/*.md`: nested directories are invisible to the old harness (good: coexistence is
  safe), and the new installer must copy recursively (`10.1`).
- The old placeholders are `{{UPPER_CASE}}`, the new ones `{{name}}`; no conflict, but a reviewer reading both should not
  expect `{{` checks of the old suite to cover the new files.
- Who writes the harness texts (run prompt, resume, wake, bounces, fix pass, review, PR body, notifications, questions
  instructions)? "Nothing is ported" means they are authored anew, about the 1,260 lines of `prompts/*.md` plus the
  inline texts of `bin/agent`. `2.3` creates only the role prompts and the catalog; each later item should add the
  prompts it uses (with the catalog test), and say so in the item text, or `6.1` will carry the whole authoring.
- Missing inputs that the old prompts receive: the hardware notes file, the language for GitHub-facing text
  (`REPO_LANGUAGE`, separate from the human-facing language of `2.1`), and the commit trailers
  (`Agent:`, `Agent-Model:`, `Task:`).

#### M-7 Other unspecified behaviour in the brief

- `Coordinator: NAME` that is not among the requested agents: refused or added? A named worker is refused (stated),
  a named investigator in a team that then leaves fewer than two outside: refused (stated); the middle case is not.
- `--coordinator` and a `Coordinator:` line that disagree: "explicit line … or `--coordinator` wins" — which of the two?
- `agent run a b "text"` versus `agent run a b -f file`: positional agents then text; a text that equals an agent name,
  and one agent with no text, need a defined error. `clap` can express it; the plan should say it.
- "the next item starts when the previous is merged (with `--no-merge`: approved by both reviews)": with `--no-merge`,
  the next item's branch is cut from an unmerged base, so its PR will contain the previous item's commits or conflict
  after the human merges. Specify: stack on the previous branch (PR base = previous branch) or wait.
- "Changes requested goes back at most 5 times": the counter is per PR, resets on a human answer? Say.
- Review fix passes use the old default `VERIFY_MAX_FIX_PASSES=5`; fine, but the plan's variable name in `agents.toml` and
  its migration of the old comments (README) should be named.

### Low

- **Parser fixture.** The plan says this file "is a test fixture that must parse". It contains lines that a naive parser
  would take for syntax: `Coordinator: NAME` inside a bullet of the brief (not at the top), numbered-looking
  prose, a four-space indented code block in the brief (the "Target layout") that looks like item description lines, and
  `Depends on:` with a list (`3, 4, 5`) and `none`. State in `docs/task-list.md` where the header ends (first
  `N. Title` line) and test exactly those four cases.
- The `agentbus` usage text omits four subcommands (H-2); worth a follow-up PR to `agentbus` outside this plan.

## 4. What is good (keep)

- One owner of the state and a socket for clients removes the file races the Bash version patches with `flock`.
- The approved dependency list is realistic: every item I traced fits it, except the SSE stream (M-4).
- `agentbus` as an unchanged process is the right call, and `4.2` tests against the real binary.
- Splitting features by "Depends on" with items taken in order is a clear rule; the three ordering defects are easy to fix.
- The hard "no prompt text in `.rs`" rule and the strict `{{name}}` renderer keep model-facing text reviewable.
- Real CLI fixtures and a test per outcome kind in `3.1` are the right way to pin the limit/quota behaviour that
  took the Bash version many patches.

## 5. Proposed edits to the plan, in task order

Minimal, item by item; the renumbering is kept small on purpose.

- **Before `1.1`** (a "Preconditions" block of human steps — section 8).
- **`1.1`** — add: the Rust test-location convention (B-3/B-4); `[workspace.lints]` source named by file path; the Rust
  checks wired into `lint`/`unit-go` or the human edits the workflow (B-2); the conditional step in `tools/gate.sh`;
  the Rust section of `AGENTS.md` names both rule files and says the reviewers apply `code-review.md`; the PR is
  merged by the human. Move the vocabulary to a new `1.2`.
- **`2.x`** — make `2.1` config + task list; `2.2` prompt catalog + CLI contract (old `2.3`); `2.3` state + protocol +
  questions (old `2.2`); each "Depends on: 1" with `2.3` also on `2.2`. Add to `docs/state.md` the ownership matrix of H-2.
- **`3.1`** — add reviewer selection with the definitions of M-1 and the deadlock test; the review verdict set;
  `Coordinator:` edge cases (M-7); split outcome classification and the round machine from formation and assignment if
  the estimate holds.
- **`4.1`** — split (H-5). **`4.2`** — GraphQL for ready or open non-draft; `COMMENT` reviews only with a byline;
  merge with `sha` and `merge_method`; pass `--agent-cmd` and the wake command to `agentbus`; list the undocumented
  `board-add`, `board-claim`, `claim-lead`, `delegate` calls and pin them with tests.
- **`5.1`** — split; state the permission/sandbox flags and the environment each adapter gives the run; build the
  `agentbus serve` arguments of H-1 in `4.2` and hand them to the adapter as data.
- **`6.1`** — split (H-5); add the three hidden compat subcommands, the run token (M-3), restart semantics (M-2),
  the catch-up scan of the bus messages.
- **`7.x`** — take the gate service first; `7.2`/`7.3` independent; add the missing commands (H-7) and worktree cleanup.
- **`8.1`** — split; add conflicts, CI/branch protection, post-merge gate, idempotence, `--no-merge` stacking (H-3, M-7).
- **`9.x`** — `9.1` on `6`, `9.2` on `7.2`; DEP-1 for the SSE stream; decide the tagged-device allow-list.
- **`10.0` (new)** — end-to-end rehearsal with stubs in a scratch home. **`10.1`** — files only; the switch and the
  rollback are a human procedure; record dropped capabilities. **`10.2`** — human action after a soak; list the files
  (including those in H-6).

## 6. Suggested content for the Rust section of `AGENTS.md` (for the human or a later round; I may not edit it)

- Where Rust tests live (B-3/B-4) and that `crates/*/tests/**` are "existing tests" for Never rule 1.
- The new checks and how to run one crate: `cargo test -p agent-core`, `cargo clippy --workspace --all-targets -- -D warnings`.
- That reviewers apply `prompts/investigator/code-review.md`.
- The relaxed size limit of this plan and the `ROUND_EXCLUDE` additions it relies on.
- That the old harness's `agent allow` is needed for `AGENTS.md` and `.github/` changes.

## 7. Risks I did not resolve

- I could not read the two rule files, so rule-ID compliance of the item texts is untested.
- Size estimates are judgement; the ratio Rust/Bash of 2 is the assumption to challenge. The first `1.1` PR is a cheap
  calibration: if it comes out over 3500 lines, scale the other estimates.
- GitHub behaviour (request-changes on one's own PR, branch protection response) is from documentation and
  experience; the fake server of `4.2` should be checked against one real PR by the human.
- CI cost on a 2-core runner was not measured.

## 8. Human actions before `1.1` (checklist)

- [ ] Commit `prompts/agent/rust-developer.md`, `prompts/investigator/code-review.md` (and the worker and investigator
      variants) to `main`.
- [ ] `agent allow <team> AGENTS.md` (and `.github/workflows/test.yml` if CI is edited by the plan); expect to merge `1.1`
      by hand (`AUTO_MERGE_HUMAN_PATHS`).
- [ ] In `~/agents/agents.conf`: `ROUND_EXCLUDE="*_gen.go go.sum package-lock.json vendor/ Cargo.lock tests/ tests.rs fixtures/"`
      and `ROUND_MAX_LINES_REPO[team-agents]=5000` for the duration of the plan.
- [ ] Decide Q1–Q9 below.

## 9. Questions for the human

Recommended answers in brackets; I did not write a `questions-1.md`, because none of these blocks the investigation.

1. Where are the rule files, and who commits them? [the human, to `main`, before `1.1`]
2. CI: wire the Rust checks into `make lint` / `make unit-go` (no workflow change), or will the human edit the workflow?
   [wire into the Makefile targets; check CI after the first push]
3. Rust test convention and harness settings of B-3/B-4: tests in `tests/` directories only, or one PR to `bin/agent`
   for `#[cfg(test)]`? [tests in `tests/` directories, no harness change]
4. Fixtures of real CLI output: will the human supply sanitized samples (including real quota and rate-limit messages),
   or authorise a one-time recording run? [human supplies samples, kept under `crates/**/tests/fixtures/`]
5. May the new `agent` implement `_wake`, `_dispatch-helpers` and `_board_done_check` as hidden compat subcommands
   (H-1), given `agentbus` cannot change? [yes]
6. At `10.2` the investigation teams, remote nodes, `agent live`, archiving and token accounting disappear. Accept, or
   keep `bin/agent` until they are ported? [accept only the ones the human no longer uses; keep `bin/agent` otherwise]
7. New default `AGENTS_HOME` for the Rust system, and the rule that cutover is a human procedure with a rollback?
   [a new directory; yes]
8. A daemon restart: accept "all runs resume", or `KillMode=process` with adoption of live runs? [accept in `6.1`,
   revisit if upgrades are frequent]
9. Dashboard: allow tagged tailnet devices as the old web does, or logins only? [logins only, documented]
10. DEP-1 for SSE (`futures-core`, `tokio-stream`): approved in advance? [yes, both are tiny and already in `axum`'s tree]

## 10. Files read

`AGENTS.md`/`CLAUDE.md`, `Makefile`, `tools/gate.sh`, `tools/test-lock.sh`, `.github/workflows/test.yml`, `install.sh`,
`bin/agent` (precheck, rules, round size, merge, quota, CLI dispatch, `bus_args_json`, `usage`), `agentbus/main.go`,
`collab.go`, `team.go`, `prompts/review.md`, `prompts/clarification.md`, `live/web*.go`, `docs/agent-inventory.md`,
`docs/tdd.md`, `rewrite-agent-in-go.md` (the earlier Go plan, for calibration), `agent/` (the unfinished Go port).
