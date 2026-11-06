#!/usr/bin/env bats
# A device shows a notification's first line first: its title. The icon of its status — and of its note, when that
# starts with another one — now comes first in it ("🟣 repo", "❌⛔ repo"), so that the kind is seen at once; the
# body is as before.

load helpers

# task_meta id — a task the notifications are about
task_meta() {
  mkdir -p "$TASKS/$1"
  printf 'ID=%s\nREPO_NAME=repo\nAGENT=lead\nTYPE=claude\nRUN=1\nKIND=worker\n' "$1" > "$TASKS/$1/meta.env"
}

@test "a finished task's notification starts with its status's icon" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  wait_grep "Status:" "$AGENTS_HOME/notifications/$t-r1-summary.md"   # written just after the status
  [ "$(head -n 1 "$AGENTS_HOME/notifications/$t-r1-summary.md")" = "✅ repo" ]
}

@test "the status's icon and the note's, when another, come first; a merge's is its own" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  task_meta P
  notify P failed "" "⛔ autopilot stopped: merging PR #7 failed"
  [ "$(head -n 1 "$NOTIFY_DIR/P-r1-summary.md")" = "❌⛔ repo" ]
  notify P merged "" "PR #7 merged by --autopilot"
  [ "$(head -n 1 "$NOTIFY_DIR/P-r1-merged-summary.md")" = "🟣 repo" ]
}

@test "with no icon in the status nor the note the title is as before" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  task_meta Q
  notify Q stopped "" "stopped by the human"
  [ "$(head -n 1 "$NOTIFY_DIR/Q-r1-summary.md")" = "repo" ]
}

@test "a notification outside a task's cycle starts with the icon its text starts with" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  push_text repo "⛔ --verify: the team could not be resumed" "why"
  f=$(ls -t "$NOTIFY_DIR"/*-summary.md | head -n 1)
  [ "$(head -n 1 "$f")" = "⛔ repo" ]
  push_text repo "--verify: a plain line"
  f=$(ls -t "$NOTIFY_DIR"/*-summary.md | head -n 1)
  [ "$(head -n 1 "$f")" = "repo" ]
}
