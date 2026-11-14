# `agent log`: the story of a round

`agent log` tells what happened from [the history](history.md). It opens `history.db` read-only and asks no daemon, so it
works when the daemon is stopped, crashed or never started. This page is about the story; the daemon's own log (8.6) and
questions, search and following (8.7) come with their items.

```sh
agent log --round round-20261114-164520-ab12        # one round, from its item to its end
agent log --team  team-20261114-164000-cd34         # the stories of the team's rounds, in order
agent log --round <id> --prompts                    # every prompt whole, not its first lines
agent log --round <id> --no-steps                   # without the steps of the runs
agent log --round <id> --json                       # the records as JSON lines
```

Exactly one of `--round` and `--team` is given. `--prompts` and `--no-steps` change the text only.

| Exit | Meaning |
|---|---|
| `0` | the story was printed |
| `1` | the history cannot be read (no file, a newer version, a row that breaks its rules), the arguments are wrong, or the output cannot be written |
| `3` | the history has no such round or team |

## The text

A story is made by `agent_core::story::render`, a pure function from the records of the round (or team) to lines; nothing
else is read. Its parts, in the order of time (records with the same time keep the order they were written in):

1. the round and its team, with its item in full;
2. the agents by role, and how each was chosen;
3. every run: who, in which role and why it started, its task, worktree and branch, how long it took, its prompt (the
   first lines and the size, or whole with `--prompts`), its command line, its steps numbered with kind and time, and how it
   ended with its final message;
4. after each run, its changes: the commits, the files added, deleted, modified and renamed with their lines, and what was
   left uncommitted;
5. the gates and the checks before a review; the pull request from opening to merge or hand-over (the body whole); every
   review pass with the reviewers' runs, each verdict and report whole; every fix pass with the findings it sent the team; the
   questions asked and the answers; the notifications; the controls; the end of the round.

A team's story is the stories of its rounds, in the order of their records.

A record that could not be kept is not silently missing: where it would stand, the story says that it was lost, what
it was and why (the cause the writer gave). A text that was cut at the limit of the history says so after it.

## `--json`

One JSON object a line, each record of the round exactly once, in the order it was written:

```json
{"sequence":41,"at":"2026-11-14T16:45:20Z","team":"team-…","rounds":["round-…"],"kind":"review_submitted","cut":[],"record":{"…":"…"}}
```

`kind` is the kind of the record: `team`, `round`, `run`, `launch`, `step`, `run_end`, `changes`, `session` or one of
the kinds of an [event](history.md#events). `record` holds every field whole; `--prompts` and `--no-steps` do not apply.
`agent log --round <id> --json | wc -l` is the number of the round's records in the database
(`SELECT count(*) FROM history_record_rounds WHERE round = '<id>'`, plus the one record of its team, the start of the team).

The text of a story is pinned by golden files under `crates/agent/tests/golden/`, each made by a daemon run with fake CLIs and
a fake GitHub server, IDs and times normalised.
