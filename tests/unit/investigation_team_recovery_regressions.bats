#!/usr/bin/env bats
# Tests for review findings R1 and R5 (inv-kimi review):
# R1: Stopped investigation parents must release leases, stop owned work, and not tick.
# R5: Non-decimal persisted seeds must hand off to human as failed instead of queueing indefinitely.

load helpers

# field task_id key decodes the shell-quoted metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# meta_put task_id key value sets or replaces KEY=value in a task's meta.env.
meta_put() {
  local file="$AGENTS_HOME/tasks/$1/meta.env"
  local val
  printf -v val '%q' "$3"
  if grep -q "^$2=" "$file"; then
    sed -i "s|^$2=.*|$2=$val|" "$file"
  else
    printf '%s=%s\n' "$2" "$val" >> "$file"
  fi
}

# free_investigators lists the investigators that are currently free.
free_investigators() {
  (
    source "$REPO_ROOT/bin/agent"
    free_investigator_list
  )
}

# configure_investigators sets up inv and inv2 in agents.conf.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' \
    "$AGENTS_HOME/agents.conf"
}

# independent_setup configures two investigators and a fake CLI in this test's temporary home.
independent_setup() {
  configure_investigators
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  printf '%s\n' 'Independent topic' > "$HOME/topic.md"
}

# create_parent_task id team base phase seed sets up an investigation_team coordinator task with standard fields.
create_parent_task() {
  local id=$1 team=$2 base=$3 phase=$4 seed=${5:-}
  local dir="$AGENTS_HOME/tasks/$id"
  git -C "$HOME/repo" config user.name 'Test'
  git -C "$HOME/repo" config user.email 'test@example.com'
  mkdir -p "$dir"
  cat > "$dir/meta.env" << EOF
ID=$id
AGENT=investigation-team
KIND=investigation_team
STATUS=running
TYPE=internal
MODEL=
EXTRA_ARGS=
TEAM=
TIER=normal
SESSION_ID=
RUN=0
ROUND=0
RUNNER_DONE=1
CREATED=$(date -Is)
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BASE=$base
INVESTIGATION_PIPELINE=1
INVESTIGATION_PHASE=$phase
INVESTIGATION_TEAM=$team
INVESTIGATION_BASE_SHA=$base
INVESTIGATION_TOPIC_SLUG=topic
INVESTIGATION_LEASE=1
INVESTIGATION_SEED=$seed
AUTO_STAGE='waiting: capacity queue'
EOF
}

# create_child_task id parent agent status creates an investigation child task for the given parent.
create_child_task() {
  local id=$1 parent=$2 agent=$3 status=$4
  local dir="$AGENTS_HOME/tasks/$id"
  mkdir -p "$dir"
  cat > "$dir/meta.env" << EOF
ID=$id
AGENT=$agent
KIND=investigation
STATUS=$status
TYPE=claude
RUN=0
ROUND=0
CREATED=$(date -Is)
PARENT_TASK=$parent
INVESTIGATION_TEAM=$parent
INV_ID=$id
EOF
}

@test "agent stop on investigation team releases lease and stops active owned work" {
  independent_setup
  local id="20261101-000030-investigation-team"
  local child="20261101-000031-inv"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 1
  create_child_task "$child" "$id" 'inv' 'running'

  cat > "$AGENTS_HOME/tasks/$id/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"$child","status":"running","commit":""},
  {"agent":"inv2","task":"task-inv2","status":"new","commit":""}
]}
EOF

  run agent stop "$id"
  [ "$status" -eq 0 ]

  # Parent status is stopped and lease is released
  [ "$(field "$id" STATUS)" = "stopped" ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]

  # Owned child is stopped
  [ "$(field "$child" STATUS)" = "stopped" ]

  # Dispatch does not tick stopped parent
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" STATUS)" = "stopped" ]

  # Investigators are now free
  run free_investigators
  [ "$status" -eq 0 ]
  [[ "$output" =~ "inv" ]]
  [[ "$output" =~ "inv2" ]]
}

@test "stopped investigation parent with lingering lease does not tick and clears lease" {
  independent_setup
  local id="20261101-000032-investigation-team"
  local child="20261101-000032-inv"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2
  create_child_task "$child" "$id" 'inv' 'running'
  # Simulate lingering lease on stopped parent with active child
  meta_put "$id" STATUS stopped
  meta_put "$id" INVESTIGATION_LEASE 1
  meta_put "$id" INVESTIGATION_HANDOFF_PENDING 1

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"$child","status":"running","commit":""},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  scenario '{"inv":{"new":"hold"},"inv2":{"new":"hold"}}'

  run agent dispatch
  [ "$status" -eq 0 ]

  # Parent did not tick to pick a merger
  [ -z "$(field "$id" INVESTIGATION_MERGER)" ]
  [ "$(field "$id" STATUS)" = "stopped" ]

  # Lingering active child is stopped
  [ "$(field "$child" STATUS)" = "stopped" ]

  # Lingering lease and handoff pending were cleared
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  [ -z "$(field "$id" INVESTIGATION_HANDOFF_PENDING)" ]

  # Capacity is free
  run free_investigators
  [ "$status" -eq 0 ]
  [[ "$output" =~ "inv" ]]
  [[ "$output" =~ "inv2" ]]
}

@test "agent stop on investigation team stops precheck unit" {
  independent_setup
  local id="20261101-000036-investigation-team"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2
  meta_put "$id" INVESTIGATION_PRECHECK running
  meta_put "$id" INVESTIGATION_PRECHECK_UNIT "agent-$id-precheck-1"

  # A live isolated unit verifies cancellation of work, beyond parent metadata changes.
  systemd-run --user --unit="agent-$id-precheck-1" bash -c \
    'printf "ready\n" > "$HOME/precheck-ready"; exec tail -f /dev/null'
  wait_grep ready "$HOME/precheck-ready"
  run systemctl --user is-active --quiet "agent-$id-precheck-1.service"
  [ "$status" -eq 0 ]

  run agent stop "$id"
  [ "$status" -eq 0 ]
  [ "$(field "$id" STATUS)" = stopped ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  run systemctl --user is-active --quiet "agent-$id-precheck-1.service"
  [ "$status" -eq 3 ]
}

@test "dispatch cancels a stopped parent precheck before taking its lock" {
  independent_setup
  local id="20261101-000037-investigation-team"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2
  meta_put "$id" INVESTIGATION_PRECHECK running
  meta_put "$id" INVESTIGATION_PRECHECK_UNIT "agent-$id-precheck-1"

  # A live isolated unit verifies cancellation of work, beyond parent metadata changes.
  systemd-run --user --unit="agent-$id-precheck-1" bash -c \
    'printf "ready\n" > "$HOME/precheck-ready"; exec tail -f /dev/null'
  wait_grep ready "$HOME/precheck-ready"
  run systemctl --user is-active --quiet "agent-$id-precheck-1.service"
  [ "$status" -eq 0 ]

  meta_put "$id" STATUS stopped
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" STATUS)" = stopped ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  run systemctl --user is-active --quiet "agent-$id-precheck-1.service"
  [ "$status" -eq 3 ]
}

@test "control: running investigation parent ticks and claims capacity" {
  independent_setup
  scenario '{"inv":{"new":"hold"},"inv2":{"new":"hold"}}'
  local id="20261101-000033-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  # While running with lease=1, inv is busy (not in free_investigators)
  run free_investigators
  [ "$status" -eq 0 ]
  [[ ! "$output" =~ "inv" ]]

  run agent dispatch
  [ "$status" -eq 0 ]

  # Running parent ticked and selected a merger
  local merger
  merger=$(field "$id" INVESTIGATION_MERGER)
  [ -n "$merger" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"merging: "* ]]

  # Prevent further advance and wait for child runner to finish
  meta_put "$id" INVESTIGATION_LEASE "''"
  meta_put "$id" STATUS waiting
  local child
  child=$(field "$id" INVESTIGATION_MERGER_TASK)
  [ -n "$child" ]
  touch "$HOME/release"
  wait_status "$child" '^(done|failed)$'
  wait_status "$id" waiting
}

@test "non-decimal persisted seed fails with handoff instead of queueing indefinitely" {
  independent_setup
  local id="20261101-000034-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2 'invalid-non-decimal-seed'

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  run agent dispatch
  [ "$status" -eq 0 ]

  # Must not be stuck in waiting: capacity queue
  [ "$(field "$id" AUTO_STAGE)" != "waiting: capacity queue" ]
  [ "$(field "$id" AUTO_STAGE)" = "waits for you: merge failed" ]
  [ "$(field "$id" STATUS)" = "waiting" ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]

  # Capacity is released, investigators are free
  run free_investigators
  [ "$status" -eq 0 ]
  [[ "$output" =~ "inv" ]]
  [[ "$output" =~ "inv2" ]]
}

@test "control: valid decimal persisted seed succeeds in picking merger" {
  independent_setup
  scenario '{"inv":{"new":"hold"},"inv2":{"new":"hold"}}'
  local id="20261101-000035-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2 '12345'

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  run agent dispatch
  [ "$status" -eq 0 ]

  local merger
  merger=$(field "$id" INVESTIGATION_MERGER)
  [ -n "$merger" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"merging: "* ]]

  # Prevent further advance and wait for child runner to finish
  meta_put "$id" INVESTIGATION_LEASE "''"
  meta_put "$id" STATUS waiting
  local child
  child=$(field "$id" INVESTIGATION_MERGER_TASK)
  [ -n "$child" ]
  touch "$HOME/release"
  wait_status "$child" '^(done|failed)$'
  wait_status "$id" waiting
}

@test "_supervise cancels precheck and clears lease for stopped parent with lingering lease" {
  independent_setup
  local id="20261101-000038-investigation-team"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2
  meta_put "$id" INVESTIGATION_PRECHECK running
  meta_put "$id" INVESTIGATION_PRECHECK_UNIT "agent-$id-precheck-1"

  # A live isolated unit verifies cancellation of work, beyond parent metadata changes.
  systemd-run --user --unit="agent-$id-precheck-1" bash -c \
    'printf "ready\n" > "$HOME/precheck-ready"; exec tail -f /dev/null'
  wait_grep ready "$HOME/precheck-ready"
  run systemctl --user is-active --quiet "agent-$id-precheck-1.service"
  [ "$status" -eq 0 ]

  meta_put "$id" STATUS stopped
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$id" STATUS)" = stopped ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  run systemctl --user is-active --quiet "agent-$id-precheck-1.service"
  [ "$status" -eq 3 ]

  # Cleanup releases both investigators for subsequent teams.
  run free_investigators
  [ "$status" -eq 0 ]
  [[ "$output" =~ "inv" ]]
  [[ "$output" =~ "inv2" ]]
}
