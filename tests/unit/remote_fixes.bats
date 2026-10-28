#!/usr/bin/env bats
# Fixes found running a team with a remote member: the wake-loop guard and remote members; --last-team and a team
# the bus does not know; a resume that fails leaves nothing half-made.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# leads — the lead's tasks, oldest first (task IDs start with their time; a glob is sorted)
leads() {
  local d
  for d in "$AGENTS_HOME"/tasks/*-lead; do [[ -d $d ]] && echo "${d##*/}"; done
  return 0
}

@test "the wake-loop guard watches local team members, not remote ones (their work is on the lead's bus)" {
  source "$REPO_ROOT/bin/agent"
  export TEAM=T REMOTE_TEAM=""   # what the runner has loaded from the task's meta
  wake_loop_applies sleeping
  run ! wake_loop_applies 'done'
  export REMOTE_TEAM=1
  run ! wake_loop_applies sleeping
}

@test "--last-team skips a team the bus does not know (left by a resume that failed)" {
  scenario '{"lead": {"new": "done", "resume": "done"}, "member": {"new": "done", "resume": "done"}}'
  agent run --team lead,member "first team"
  wait_status "$(task_of lead)" 'done'
  real=$(meta "$(task_of lead)" TEAM)
  mkdir -p "$AGENTS_HOME/tasks/20991231-000000-lead"
  printf 'ID=20991231-000000-lead\nAGENT=lead\nTEAM=20991231-000000-team\nSTATUS=stopped\nREPO_ROOT=%s\n' "$HOME/repo" \
    > "$AGENTS_HOME/tasks/20991231-000000-lead/meta.env"
  cd "$HOME/repo"
  run agent run --last-team "go on" < /dev/null
  [ "$status" -eq 0 ]
  [[ $output == *"$real"* ]]
}

@test "a resume that fails leaves no task or worktree behind; carried changes are committed on their branch" {
  scenario '{"lead": {"new": "done", "resume": "done"}, "member": {"new": "done", "resume": "done"}}'
  agent run --team lead,member "first team"
  wait_status "$(task_of lead)" 'done'
  team=$(meta "$(task_of lead)" TEAM)
  branch=$(meta "$(task_of member)" BRANCH)
  echo "half-done" > "$(meta "$(task_of member)" WORKTREE)/wip.txt"   # uncommitted work to carry
  before=$(ls "$AGENTS_HOME/tasks" | wc -l)
  ab=${AGENTBUS_BIN:-$HOME/.local/bin/agentbus}; [[ -x $ab ]] || ab=$(command -v agentbus)
  printf '#!/usr/bin/env bash\n[[ $1 == team-create ]] && { echo "team-create: refused" >&2; exit 1; }\nexec %q "$@"\n' \
    "$ab" > "$BATS_TEST_TMPDIR/agentbus-failing"
  chmod +x "$BATS_TEST_TMPDIR/agentbus-failing"
  cd "$HOME/repo"
  AGENTBUS_BIN="$BATS_TEST_TMPDIR/agentbus-failing" run agent run --resume "$team" "go on" < /dev/null
  [ "$status" -ne 0 ]
  [ "$(ls "$AGENTS_HOME/tasks" | wc -l)" -eq "$before" ]          # no new task left
  run git worktree list                                             # the branch is checked out nowhere
  [[ $output != *"[$branch]"* ]]
  git log -1 --format=%s "$branch" | grep -q '^WIP: uncommitted work carried by an interrupted resume'
  git show "$branch:wip.txt" | grep -q half-done
  run agent run --resume "$team" "go on" < /dev/null                # nothing blocks the branches: resuming works
  [ "$status" -eq 0 ]
}
