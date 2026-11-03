# --autopilot reviews: only two investigators rotate; the review cap — inv-opus

Base: `52c4f19`. Branch: `agents/inv-opus/20261103-095950`.

## Summary

1. **Rotation — a real bug, fixed (opt-in mode, see "Why opt-in").** `start_review_pair` (`bin/agent`)
   deliberately keeps a PR's previous pair for its re-reviews ("each re-checks its own findings"), and draws a new
   pair at random only for a new PR. Over a round with fix passes the same two investigators review three times in
   a row; across PRs the random draw has no memory, so one investigator can appear in pair after pair. The fix
   adds `AGENT_REVIEW_DRAW=rotate`: a pair is never the pair of either of the two reviews before it while the free
   investigators can make another, the two that reviewed least recently come first, then a pair not seen yet.
   With four investigators A–D, all free: A+B, C+D, A+C, B+D, A+D, B+C — every pair once, everyone three times.
2. **The 3-review stop — already in place, it worked; no change.** `VERIFY_MAX_FIX_PASSES=2` (the default and
   your `agents.conf`) gives work → review → fix → review → fix → review → stop ("waits for you", a PR comment
   mentioning you, `review_fixes_exhausted`). The logs show it stopping on PR #43; the three further reviews of
   #43 came after the team was resumed by hand, which starts the count anew (details below).

## Evidence from this installation (read only)

`~/agents/agents.conf` has four investigators: `inv-opus`, `inv-astra`, `inv-kimi`, `inv-gemini-argon`, and no
`AGENT_REVIEW_DRAW` (so `random`). Last pairs per PR (`~/agents/reviews/*/A.agent B.agent`):

| PR | pair (last pass) |
|---|---|
| 24, 29, 35 | inv-astra, inv-kimi |
| 30–34, 43 | inv-astra, inv-gemini-argon |
| 37 | inv-kimi, inv-opus |
| 40 | inv-gemini-argon, inv-kimi |

`inv-astra` is in 9 of 11 pairs; `inv-opus` in one. Within a PR the pair never changed except when a review
failed (PR #43: astra+kimi four times, then astra+gemini after kimi ran out of quota). `inv-opus` is also the
investigator most often busy with investigations like this one — a busy investigator is not free and cannot be
chosen; the rotation takes it as soon as it is free (it has reviewed least recently).

PR #43, from `logs/agent.jsonl` (UTC):

| time | event |
|---|---|
| 04:43 | pair 1: needs-fixes ×2 → `review_fix_pass` pass=1 |
| 05:15 | pair 2: needs-fixes ×2 → pass=2 |
| 05:52 | pair 3: needs-fixes ×2 → **`review_fixes_exhausted` passes=2** — the autopilot stopped |
| 06:55 | `run` (not `_fixpass`/`_merge`): a manual `agent run --resume … --autopilot`, after a mistyped one at 06:55:44 → `FIX_PASS=0` |
| 07:21, 07:55 | two fix passes of the new run |
| 08:14 | both ready-to-merge → `autopilot_merged` pr=43 |

So the cap counts per `--autopilot` run; a resume by the human is the human's decision, and it gets its own three
reviews. If you want the count to carry over a manual resume, that is a separate decision (question 2 below).

What is not counted as a review (by design): a review done again because it gave no verdict (failed run, quota),
and a return for the gate/tests before the review (`AGENT_PRECHECK_MAX`).

## The change

`bin/agent`:

- `AGENT_REVIEW_DRAW=rotate` in `start_review_pair`: the pair comes from `rotate_review_pair` instead of the
  previous pair of the PR plus a random draw. Score per candidate pair of free investigators (lower wins):
  tier (2 = the last pair, 1 = the one before, 0 = otherwise), then the sum of the two investigators' last-use
  positions in the history, then the pair's own last-use position (never = 0), then the list's order.
  With only two free investigators the pair is those two even if they were the last pair (it never waits for a
  third, as before).
- `remember_review_pair` / `review_pair_history`: every started pair (any draw) is appended to
  `~/agents/review-pairs` ("a b" per line, last 20 kept, written to a temp file and renamed; called under the
  dispatch lock, as `start_review_pair` is). A missing file is an empty history — existing installations need
  nothing.
- `start_review_slot`: a re-review whose slot's previous review was written by another investigator (what
  rotation causes) gets it as "The previous review of this PR, by <name>" instead of "Your previous review".
- The defaults are unchanged (`random`); `config/agents.conf` (the template) sets `AGENT_REVIEW_DRAW=rotate`;
  README ("--autopilot", the fix loop, Configuration, Files and state).

**To use it here:** add `AGENT_REVIEW_DRAW=rotate` to `~/agents/agents.conf` after the merge (I did not touch
`~/agents/`).

### Why opt-in and not the new default

`tests/unit/review_pair_random.bats` "a PR's next review pass keeps its pair" pins the sticky behaviour under
the default draw. AGENTS.md rule 1 forbids changing existing tests without your explicit approval, so `random`
stays the script's default and the existing test still passes. If you want rotation to be the default, approve
removing (or rewriting) that one test and changing `AGENT_REVIEW_DRAW:-random` to `:-rotate` — a one-line change.

## Tests

New `tests/unit/review_pair_rotate.bats` (added to `SHELL_SCRIPTS` for shellcheck), red commit first:

- four free: A+B, then C+D, then one of each;
- six pairs: none equal to either of the two before, the six possible pairs, each investigator three times;
- the same PR's next pass gets another pair;
- three free: a+b, a+c, then b+c;
- an investigator busy during the last pairs is chosen once free;
- two free: that pair, even twice in a row;
- the history keeps 20 lines and records any draw;
- end to end (`--autopilot`, four stub investigators, all `review-fixes`): three review passes, each by a
  different pair, all four investigators involved, `review_fixes_exhausted` and the PR mention, no fourth pass;
  the third pass's re-reviews name the previous reviewer.

`make lint test`: exit 0, zero findings, no failing test (Go, bats; the new file included).

## Not done / open questions for the merger and the human

1. Make `rotate` the default? Needs approval to change the existing test above.
2. Should a manual `agent run --resume … --autopilot` continue the review count instead of resetting it
   (e.g. `FIX_PASS` carried over when the resumed task's `REVIEW_RESULT` was `fixes` and the cap was hit)? Today
   it resets; the stop itself works.
3. A review done again after "no verdict" (`retry_review_slot`) is not recorded in the history — it takes the
   first free investigator outside the pair, as before.
4. Suggested AGENTS.md note: none needed.
