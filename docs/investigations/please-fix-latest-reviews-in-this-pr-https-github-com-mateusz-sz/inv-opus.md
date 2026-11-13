# The latest reviews of PR #98 — `inv-opus`

Task `20261113-200949-inv-opus`, branch `agents/inv-opus/20261113-200949`, based on the PR's head
[`f1712ab`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f1712abedf40407422df82e6d4ff7b89d9890eaf).

## Summary

- [#98](https://github.com/mateusz-szczyrzyca/team-agents/pull/98) has two reviews, both "Needs changes", `8` major
  findings in all:
  - [`inv-kimi-deepinfra`](https://github.com/mateusz-szczyrzyca/team-agents/pull/98#pullrequestreview-5896118000), at `b6bacd4`;
  - [`inv-astra`](https://github.com/mateusz-szczyrzyca/team-agents/pull/98#pullrequestreview-5896408044), at `a865c09`.
- Fix passes 1 and 2, already on the PR, address all `8`. I checked each fix against its finding in the code and the
  tests at `f1712ab`. Each one holds. One fix is not complete, and the description has one defect:
  1. **Finding 3 of `inv-astra` (`#[expect(…)]` must be disclosed) was only half fixed.** The form PR-5 prescribes for a
     suppression that only some builds need is `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`. It was still
     no marker, and neither was `#[cfg_attr(miri, ignore)]`, which skips a test. Both are now fixed here, test first.
  2. **The PR description does not name the `TEST-FIX:` commit of pass 2 under `## Tests changed after RED`.** That
     section says "these three `TEST-FIX:` commits" but lists two. The bullet for
     [`f84c70c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f84c70c05ee4e3552aa90b0d7396e483a3acc310)
     ended up inside "Review fixes (pass 1)", item 2. Under TDD-14, and under item 8.2's own rule, which only reads
     that section, `f84c70c` is therefore an unnamed edit of a $\color{red}{\textbf{RED}}$ test. The fix is a change of
     the description: see "For the merger".
- The comment "merge failed on PR #98" came after the precheck gate's `rust-lld` `SIGBUS` on a full tmpfs `/tmp`. The PR
  itself is `MERGEABLE` / `CLEAN` against `main` ([`caa41ea`](https://github.com/mateusz-szczyrzyca/team-agents/commit/caa41ea6baa4e9f58a1573bdb633805f3493e2cc)),
  and `main` is an ancestor of the branch. I found no defect in the code behind that failure. The gate passes here.

## The findings and their fixes at `f1712ab`

| Review | Finding | Fix | Verdict |
|---|---|---|---|
| `inv-kimi-deepinfra` 1 | merges hid edits of a $\color{red}{\textbf{RED}}$ test | [`cfb9032`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cfb90329492cd371f78a5b577b4f332a6dfb44cf) / [`f8809a0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f8809a0c62ece8a6719a94d9196b2adf97a081dd) | holds: a merge is read against each parent (`merge_files`), real-repository regression |
| `inv-kimi-deepinfra` 2 | chmod-only edit of a test passed | [`7494f4b`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7494f4ba4265160039ac0f597cc05c39f7783b0c) / [`6cfdfaa`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6cfdfaa49d0ce71e3eed1a291cc24fa69f750ecc) | holds: `TestChange::ModeChanged`, for the human |
| `inv-kimi-deepinfra` 3 | four daemon tests after their behaviour | history reordered: [`d46b7f4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d46b7f4b00ba5d2d1a3217bc3d615b3759923687) before [`39a4225`](https://github.com/mateusz-szczyrzyca/team-agents/commit/39a4225ae2eda168ca7e1042bfdcb0b534cdd6bc) | holds |
| `inv-kimi-deepinfra` 4 | test edit outside a `TEST-FIX:` | [`0800b00`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0800b00e81a04e186486a3c7bbe932bb9c88ecb3) touches no test file | holds |
| `inv-astra` 1 | answers matched by position | [`73dff3d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/73dff3dc9795e9366ed3de43f0e351fb724bcdc9) / [`cead9d0`](https://github.com/mateusz-szczyrzyca/team-agents/commit/cead9d0dfe9983d6ce28c04e5397fdee07172283) | holds: subject `n` is question `n + 1`, as `questions` numbers them; `decisions` is the only reader ([`owner/rounds/checks.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/640a1b5f641d9de52f834b9b8d76b066e731751c/crates/agent/src/daemon/owner/rounds/checks.rs#L116)); duplicate numbers are refused by the parser |
| `inv-astra` 2 | mode change with added lines passed | [`3eef7f8`](https://github.com/mateusz-szczyrzyca/team-agents/commit/3eef7f8ada2e4841c117ef4a2b3aa7c5399aeba7) / [`ed26057`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ed2605745cdcc16e4f7901997a94e481181fa986) | holds: `git diff --raw -z -M` uses the same rename detection as the name-status/numstat diff, so the paths agree |
| `inv-astra` 3 | `#[expect(…)]` needed no disclosure | [`f84c70c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f84c70c05ee4e3552aa90b0d7396e483a3acc310) / [`1b261e3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1b261e308acff744e32d6f41a00cd1de0b7d5dd8) / [`f583ad2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f583ad24ef9610f56fbc3367a633ee06ad34f09f) | **partly:** only at the start of a line; `cfg_attr` forms missed (fixed below); `f84c70c` not named in the description |
| `inv-astra` 4 | substring disclosure | [`823254d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/823254d961e69467b1c425e8d8961144eaa4c151) / [`ea8a8a5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ea8a8a5bcbfe0928e35cf2536fa89da20e565815) | holds: `names_path` with path-character boundaries |

## Commits on this branch

- [`1fb4455`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fb44553b8d20c4218181fe5960e44dddda9825d) — `TEST-FIX:`
  in [`markers/tests.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/640a1b5f641d9de52f834b9b8d76b066e731751c/crates/agent-core/src/decide/checks/markers/tests.rs).
  [`1b261e3`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1b261e308acff744e32d6f41a00cd1de0b7d5dd8) inserted
  the expect test between the comment that states the rule of the shell and bats markers and that rule's test. The
  comment then stood above the wrong test (DOC-3). Only those two comment lines move back. No assertion changes.
- [`860a982`](https://github.com/mateusz-szczyrzyca/team-agents/commit/860a982b37713013baa4440280abedd0741be4b7) —
  $\color{red}{\textbf{RED}}$: an attribute that `cfg_attr` applies is a marker: `allow(`/`expect(` suppress a lint and
  `ignore` skips a test, both single-line and wrapped by rustfmt (`#[cfg_attr(` alone on its line, up to `)]`). The
  nearest cases that are not markers: `derive`, `doc(cfg(feature = "ignored"))`, `my_allow(`, a comment, a string, a
  line after the `)]`, a line after a gap. Failing output in the message: `3` new tests failed on their assertions,
  `58 passed; 3 failed`.
- [`e1dc5ab`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e1dc5abf10610610dd14cb04a0206a5048051313) —
  $\color{green}{\textbf{GREEN}}$: `Place::Applied` and the two `RUST` rules in
  [`markers.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/640a1b5f641d9de52f834b9b8d76b066e731751c/crates/agent-core/src/decide/checks/markers.rs).
  `markers_in` reads the added lines of one file together, so a wrapped attribute is read once more as one line, at
  the number of its first. `markers_added` in
  [`checks.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/640a1b5f641d9de52f834b9b8d76b066e731751c/crates/agent-core/src/decide/checks.rs)
  takes the lines of each file together (`chunk_by` on the path).
- [`7d419cc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7d419cce3f96eb17f8e5a5eb86c31bbf59c11e2e) —
  $\color{red}{\textbf{RED}}$: a word inside a string of a `cfg_attr` (`doc = "ignore this, allow(x) …"`) is no marker,
  and an escaped quote does not end the string. `61 passed; 1 failed`.
- [`76164ca`](https://github.com/mateusz-szczyrzyca/team-agents/commit/76164ca6aaaac27e50647035adc87a6115516eaf) —
  $\color{green}{\textbf{GREEN}}$: `ends_in_string`.
- [`640a1b5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/640a1b5f641d9de52f834b9b8d76b066e731751c) — the
  rule's row in [`docs/architecture.md`](https://github.com/mateusz-szczyrzyca/team-agents/blob/640a1b5f641d9de52f834b9b8d76b066e731751c/docs/architecture.md).

TDD-2: I broke each new branch once and saw its tests fail, then restored it. The three branches are the stop at
`)]`, the stop at a gap, and the escape in a string. I first wrote the string check without a test that asked for it.
I took it out of the first $\color{green}{\textbf{GREEN}}$ commit and gave it a cycle of its own.

## Checks

- `GATE_WAIT=1 make gate` at [`640a1b5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/640a1b5f641d9de52f834b9b8d76b066e731751c)
  printed `gate: PASS at 640a1b5`: `2,011` tests passed (`2,005` at `f1712ab`, plus `6`), `0` failed, `0` ignored, and
  `0` warnings in the log.
- `cargo test -p agent-core --lib decide::checks`: `62 passed; 0 failed`.
- TDD-14 freeze over the whole branch (`origin/main..HEAD`): for every $\color{red}{\textbf{RED}}$ commit, I listed the
  later commits that delete lines in its test files. Only `TEST-FIX:` commits do, and each touches test files only:
  - [`0dcd4f2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/0dcd4f29ef1a288247f107dfe524b7d651a938e9) (`−1`) — named in the description;
  - [`ff8e2e5`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ff8e2e536c0521b6780de52686dddf853ab214d7) (`−1`) — named;
  - [`f84c70c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f84c70c05ee4e3552aa90b0d7396e483a3acc310) (`−3`) — **not named under the heading**;
  - [`1fb4455`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fb44553b8d20c4218181fe5960e44dddda9825d) (`−2`) — this branch, to be named.

  My two $\color{red}{\textbf{RED}}$ commits: `git diff <red> HEAD -- <its test files>` shows `0` removed lines.
- Size by the PR-1 command: `3,635` changed lines outside tests over `origin/main...HEAD`, under the relaxed `5,000`.
  That count includes the findings files under `docs/investigations/`, which the description's `3,313` leaves out.
  This branch adds `138` of those lines over `f1712ab`: code and docs, without this file.

## For the merger: the description of #98

1. Under `## Tests changed after RED`, the list should hold four commits ("these four"):
   - move the [`f84c70c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/f84c70c05ee4e3552aa90b0d7396e483a3acc310)
     bullet there from "Review fixes (pass 1)", item 2, where it is now;
   - add [`1fb4455`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1fb44553b8d20c4218181fe5960e44dddda9825d):
     the comment stating the rule of the shell and bats markers is moved back above its test, after `1b261e3` had
     separated them. Two comment lines, no assertion.
2. Add to "Review fixes (pass 2)", item 3, that attributes applied through `#[cfg_attr(…)]` (single-line and wrapped)
   are markers too: [`860a982`](https://github.com/mateusz-szczyrzyca/team-agents/commit/860a982b37713013baa4440280abedd0741be4b7)
   / [`e1dc5ab`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e1dc5abf10610610dd14cb04a0206a5048051313),
   [`7d419cc`](https://github.com/mateusz-szczyrzyca/team-agents/commit/7d419cce3f96eb17f8e5a5eb86c31bbf59c11e2e)
   / [`76164ca`](https://github.com/mateusz-szczyrzyca/team-agents/commit/76164ca6aaaac27e50647035adc87a6115516eaf).
3. `## Lint suppressions`: the marker table now also has `allow(`, `expect(` and `ignore` as texts of
   `Place::Applied`. They are still data only, in the same three files already named. No suppression is in effect.
4. `## Gate`: `2,011` tests at the new head.

## Not done, and limits

Each of these is outside what the reviews asked. None of them is a regression:

- **An existing wrapped `cfg_attr` whose lint list is edited:** only the inner line is added, and its `#[cfg_attr(`
  line is not, so no marker is seen. The checks see added lines only.
- **Raw strings** (`r#"…"#`) in a `cfg_attr` are not understood by `ends_in_string`. A quote inside one could hide or
  reveal a word. Real attributes rarely use them.
- **`#[cfg(not(test))]` and `cfg!(not(test))`** in production code are not `ChecksUnderTest` markers. Only
  `cfg!(test)` is, and TDD-12 forbids both. I left it alone: no review asked for it.
- **A renumbered question:** answers are tied to question numbers, which is what `inv-astra` asked for. If the human
  or the lead renumbers `## 2.` to `## 1.`, the permission moves with the number. Checking that a question's title
  still names its subject would close this.
- **A later commit that only changes the mode of a test a $\color{red}{\textbf{RED}}$ commit added** is no
  `RedTestEdited`: `takes_lines_out` is false for `0/0`. The mode rule only covers existing tests.
- I did not edit the PR or its description (the task says the merger integrates), did not post on GitHub, and ran no
  model CLI.
