# Review rotation and the review cap — inv-opus

Examined base: `52c4f1967207e939ffb07001f5fd3200ad1a06a8`. Branch: `agents/inv-opus/20261103-092457`.

## 1. Why only two investigators keep reviewing

Two parts of `start_review_pair` (`bin/agent`) cause it:

1. **A PR's next review pass keeps its pair.** When `reviews/<repo>-<pr>/A.agent` and `B.agent` exist and those
   investigators are free, they review again ("each re-checks its own findings"). The existing test
   `review_pair_random.bats` — "a PR's next review pass keeps its pair" — requires this. So every fix-pass
   re-review of a PR goes to the same two investigators.
2. **No memory across PRs.** For a new PR the pair is drawn at random among the *free* investigators
   (`AGENT_REVIEW_DRAW=random`). Nothing records who reviewed the previous PR, so the same pair can be drawn
   again. Investigators that are often busy (investigation teams such as this one hold `inv-opus`) are seldom
   free and are seldom drawn.

What the installation recorded (read-only: `~/agents/reviews/*`, the coordinators' `activity.jsonl`):

| PR | pairs started, in order |
|---|---|
| #40 | gemini-argon+kimi, gemini-argon+kimi, gemini-argon+kimi |
| #43 | astra+kimi ×4, then astra+gemini-argon ×2 |
| #30–#34 (final pairs) | astra+gemini-argon each time |

`inv-opus` reviewed one PR (#37) out of eleven.

## 2. The three-review stop is already there

`VERIFY_MAX_FIX_PASSES` (default 2, also 2 in the installation's `agents.conf`) is used by `do_fix_pass` for both
`--verify` and `--autopilot`: review → fix pass 1 → review → fix pass 2 → review → the team is not woken again, the
human is mentioned on the PR and notified, and the stage becomes "waits for you: the reviews still find problems
after 2 fix passes". So no code change is needed for this.

The installation's data confirms it. PR #43: the coordinator task `20261103-062800-gemini-1` (`FIX_PASS=2`) stopped
with that stage after the third pair of reviews. The next three reviews (08:15, 08:42, 09:08) came after the task
was resumed at 07:55 (`20261103-075558-gemini-1`, `RESUMED_FROM` the stopped task, `FIX_PASS=0`). A resume starts
the count anew. This is why it can look like the reviews went on by themselves.

Two details:

- A review done again because it gave no verdict (failed run, out of quota) does not count as a review.
- A return from the pre-review checks (gate or missing tests, `AGENT_PRECHECK_MAX`) is not a review either.

The test `tests/unit/autopilot_review_cap.bats` (new) pins this for `--autopilot`. Until now only `--verify` had
such a test (`verify_loop.bats`). The test runs six reviews in three pairs, then checks the stage, the mention of
the human, and that nothing more starts.

## 3. The fix: `AGENT_REVIEW_DRAW=rotate`

A new mode of the review pair, in `bin/agent`:

- `review_history_add` appends every started pair to `~/agents/reviews/history` (one line per pair, names
  sorted, oldest first, last 100 kept). It does this in every mode, so switching to `rotate` starts from the real
  history. It is written under the dispatch lock with tmp+mv. If the write fails, `review_history_failed` is
  logged and nothing else happens.
- `review_pair_pick` ranks every pair of the given investigators:
  1. *strict* excludes the last two pairs (a PR's re-review included);
  2. fewest members of the last pair;
  3. the pair used least recently, or never;
  4. its members' reviews least recent;
  5. list order.

  With A, B, C, D it gives A+B, C+D, A+C, B+D, A+D, B+C: all six pairs, each one different from the two before.
- `review_pair_rotate` picks the strict pair among the free investigators. If only a *busy* investigator could
  form such a pair, it waits `AGENT_REVIEW_ROTATE_WAIT_MIN` (20) minutes from the pair's request
  (`REVIEW_PAIR_SINCE`). After that it takes the best free pair and logs `review_pair_repeated`. If no strict pair
  exists at all (two investigators), it takes their pair at once, so the autopilot never stalls.
- `cmd__supervise` (the timer) now dispatches while a rotate pair waits (`review_rotation_waits`). Otherwise the
  20-minute fallback would depend on some run ending.
- `start_review_slot`: a re-review after a fix pass whose slot moved to another investigator gets the slot's
  previous review labelled "The previous review, by X" instead of "Your previous review". So the findings are
  still checked first, and nobody is told they wrote another investigator's review.

Tests (new): `tests/unit/review_pair_rotate.bats` has 8 tests:
- A+B then C+D, and all six pairs;
- a re-review goes to another pair;
- a history shared across PRs;
- two investigators;
- the wait and then the fallback;
- three investigators;
- the re-review prompt;
- the timer dispatch.

Docs: README ("The review rotation" under Teams, the fix-loop paragraph, the configuration table) and
`config/agents.conf`.

### The default stays `random`: a decision for the human

The request is that *every* re-review uses a different pair. That contradicts the existing test "a PR's next review
pass keeps its pair", which runs in the default mode. AGENTS.md rule 1 forbids changing that test without the
human's approval. So `rotate` is opt-in: **add `AGENT_REVIEW_DRAW=rotate` to `~/agents/agents.conf`** (I did not
edit the installation, rule 3). If you want `rotate` as the default, approve changing the default in
`bin/agent:374` and replacing that test (and the header of `review_pair_random.bats`) with its `rotate`
counterpart. The phase-2 merger can do it in a few lines.

## 4. Compatibility (rule 9)

- New, additive: the file `~/agents/reviews/history`, the configuration variables `AGENT_REVIEW_ROTATE_WAIT_MIN`
  and the new `AGENT_REVIEW_DRAW` value `rotate`, the log events `review_pair_repeated` and `review_history_failed`.
- `random` and `list` behave as before. The only difference is that the history file is written.
- No persisted format changed. A missing history means "no reviews yet".

## 5. Checks

- `make lint`: passes, no findings.
- `make test`: passes (Go tests of `agentbus` and `live`, 492 bats tests, exit 0).

## 6. Left open / suggestions

- `retry_review_slot` (a review done again for want of a verdict) still takes the first free third investigator in
  name order, and does not update the history line of the pair. That is minor: the retried pair's two original
  investigators are already in the history.
- For AGENTS.md: mention that a bats test sourcing `bin/agent` must call functions in a `( source …; … )`
  subshell, not `bash -c`. With `bash -c`, unrelated notify paths hit `RUN: unbound variable`.
