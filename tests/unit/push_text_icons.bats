#!/usr/bin/env bats
# "--verify: PR #67 still has 1 critical or major problem(s) after 5 fix passes — your decision is needed" reached
# the human with no icon: a notification outside a task's cycle (push_text) gets the icon its text starts with in
# its title, and five of them started with none. Every one now starts with its icon — ⛔ where the work stopped and
# the human decides, ⚠ where something is amiss but goes on.

load helpers

@test "every notification outside a task's cycle starts with an icon" {
  run grep -n 'push_text "' "$REPO_ROOT/bin/agent"
  [ "$status" -eq 0 ]
  local line text byte missing=()
  while IFS= read -r line; do
    text=$(sed -E 's/.*push_text "(\$\(task_field "\$id" REPO_NAME\)|[^"]*)" "//' <<< "$line")   # after the repository
    byte=$(printf '%s' "$text" | head -c 1 | od -An -tu1 | tr -d ' ')
    ((byte >= 128)) || missing+=("$line")   # an icon is no ASCII character
  done <<< "$output"
  printf 'no icon: %s\n' "${missing[@]}" >&2
  [ "${#missing[@]}" -eq 0 ]
}

@test "the end of the fix passes stops the work: its notification's title starts with ⛔" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  line=$(grep 'after $pass fix passes' "$REPO_ROOT/bin/agent")
  [[ $line == *'"⛔ --verify:'* ]]
  push_text repo "⛔ --verify: PR #67 still has 1 critical or major problem(s) after 5 fix passes — your decision is needed"
  f=$(ls -t "$NOTIFY_DIR"/*-summary.md | head -n 1)
  [ "$(head -n 1 "$f")" = "⛔ repo" ]
}
