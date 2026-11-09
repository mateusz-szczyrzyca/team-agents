#!/usr/bin/env bats
# Tests for investigation team phase 2 and 3 member selection, deterministic seeding,
# quota pause filtering, capacity queueing, single-replacement failure handling,
# and legacy phase1-only backward compatibility.

load helpers

# field task_id key decodes the shell-quoted metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# meta_put task_id key value sets or replaces KEY=value in a task's meta.env.
meta_put() {
  local file="$AGENTS_HOME/tasks/$1/meta.env"
  if grep -q "^$2=" "$file"; then sed -i "s|^$2=.*|$2=$3|" "$file"; else printf '%s=%s\n' "$2" "$3" >> "$file"; fi
}

# configure_three_investigators configures inv, inv2, and inv3 in agents.conf.
configure_three_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' \
    "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  scenario '{"inv":{"new":"hold"},"inv2":{"new":"hold"},"inv3":{"new":"hold"}}'
}

# create_parent_task id team base sets up an investigation_team coordinator task with initial phase 1 outcomes.
create_parent_task() {
  local id=$1 team=$2 base=$3
  local dir="$AGENTS_HOME/tasks/$id"
  # commit-tree fixtures need an identity independent of the caller's environment.
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
INVESTIGATION_PHASE=1
INVESTIGATION_TEAM=$team
INVESTIGATION_BASE_SHA=$base
INVESTIGATION_TOPIC_SLUG=test-topic
INVESTIGATION_LEASE=1
EOF
}

@test "deterministic seed selection is reproducible across runs with same seed and candidate pool" {
  configure_three_investigators
  local base id1 id2 dir1 dir2 c1 c2 c3 merger1 merger2
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id1="20261101-000001-investigation-team"
  id2="20261101-000002-investigation-team"
  dir1="$AGENTS_HOME/tasks/$id1"
  dir2="$AGENTS_HOME/tasks/$id2"
  create_parent_task "$id1" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  c3=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c3" "$base^{tree}")

  cat > "$dir1/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"},
  {"agent":"inv3","task":"task-inv3","status":"done","commit":"$c3"}
]}
EOF

  export AGENT_INVESTIGATION_SEED=42
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id1" INVESTIGATION_PHASE)" = "2" ]
  merger1=$(field "$id1" INVESTIGATION_MERGER)
  [ -n "$merger1" ]
  [[ "$merger1" =~ ^(inv|inv2|inv3)$ ]]

  # Prevent further advance on first parent and wait for its child to finish
  meta_put "$id1" INVESTIGATION_LEASE "''"
  local child1
  child1=$(field "$id1" INVESTIGATION_MERGER_TASK)
  [ -n "$child1" ]
  touch "$HOME/release"
  wait_status "$child1" '^done$'
  rm -f "$HOME/release"
  rm -rf "$dir1"

  create_parent_task "$id2" 'inv,inv2,inv3' "$base"
  cat > "$dir2/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-invB","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2B","status":"done","commit":"$c2"},
  {"agent":"inv3","task":"task-inv3B","status":"done","commit":"$c3"}
]}
EOF

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id2" INVESTIGATION_PHASE)" = "2" ]
  merger2=$(field "$id2" INVESTIGATION_MERGER)
  [ "$merger1" = "$merger2" ]
}

@test "investigator with failed phase1 status is never eligible for merger selection" {
  configure_three_investigators
  local base id dir c1 c3 merger
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000003-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c3=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c3" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"failed"},
  {"agent":"inv3","task":"task-inv3","status":"done","commit":"$c3"}
]}
EOF

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_PHASE)" = "2" ]
  merger=$(field "$id" INVESTIGATION_MERGER)
  [ "$merger" != "inv2" ]
  [[ "$merger" =~ ^(inv|inv3)$ ]]
  [[ "$(field "$id" AUTO_STAGE)" == *"merging: $merger"* ]]
}

@test "quota-paused investigator is excluded from merger selection even if in DONE pool" {
  configure_three_investigators
  local base id dir c1 c2 pause_task
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000004-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  pause_task="$AGENTS_HOME/tasks/20261101-000000-inv"
  mkdir -p "$pause_task"
  cat > "$pause_task/meta.env" << EOF
ID=20261101-000000-inv
AGENT=inv
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) + 3600 ))
EOF

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_PHASE)" = "2" ]
  [ "$(field "$id" INVESTIGATION_MERGER)" = "inv2" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"merging: inv2"* ]]
}

@test "when all eligible merger candidates are quota-paused the team waits in capacity queue" {
  configure_three_investigators
  local base id dir c1 c2 p1 p2
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000005-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  p1="$AGENTS_HOME/tasks/20261101-000000-inv"
  p2="$AGENTS_HOME/tasks/20261101-000000-inv2"
  mkdir -p "$p1" "$p2"
  cat > "$p1/meta.env" << EOF
ID=20261101-000000-inv
AGENT=inv
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) + 3600 ))
EOF
  cat > "$p2/meta.env" << EOF
ID=20261101-000000-inv2
AGENT=inv2
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) + 3600 ))
EOF

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_LEASE)" = "1" ]
  [ -z "$(field "$id" INVESTIGATION_MERGER)" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"waiting: capacity queue"* ]]
}

@test "failed merger is replaced once by another eligible done investigator" {
  configure_three_investigators
  local base id dir c1 c2 merger_task
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000006-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  merger_task="20261101-000006-inv"
  mkdir -p "$AGENTS_HOME/tasks/$merger_task"
  cat > "$AGENTS_HOME/tasks/$merger_task/meta.env" << EOF
ID=$merger_task
AGENT=inv
KIND=investigation
INVESTIGATION_TEAM=$id
STATUS=failed
EOF

  meta_put "$id" INVESTIGATION_PHASE 2
  meta_put "$id" INVESTIGATION_MERGER inv
  meta_put "$id" INVESTIGATION_MERGER_TASK "$merger_task"
  meta_put "$id" INVESTIGATION_MERGER_REPLACEMENTS 0

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_MERGER_REPLACEMENTS)" = "1" ]
  [ "$(field "$id" INVESTIGATION_MERGER)" = "inv2" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"merging: inv2"* ]]
}

@test "second merger failure transitions parent task to waiting for human" {
  configure_three_investigators
  local base id dir c1 c2 merger_task2
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000007-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  merger_task2="20261101-000007-inv2"
  mkdir -p "$AGENTS_HOME/tasks/$merger_task2"
  cat > "$AGENTS_HOME/tasks/$merger_task2/meta.env" << EOF
ID=$merger_task2
AGENT=inv2
KIND=investigation
INVESTIGATION_TEAM=$id
STATUS=failed
EOF

  meta_put "$id" INVESTIGATION_PHASE 2
  meta_put "$id" INVESTIGATION_MERGER inv2
  meta_put "$id" INVESTIGATION_MERGER_TASK "$merger_task2"
  meta_put "$id" INVESTIGATION_MERGER_REPLACEMENTS 1

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" STATUS)" = "failed" ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"failed: merge failed"* ]]
}

@test "failed reviewer is replaced once by another eligible team member excluding the merger" {
  configure_three_investigators
  local base id dir c1 c2 c3 rev_task
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000008-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  c3=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c3" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"},
  {"agent":"inv3","task":"task-inv3","status":"done","commit":"$c3"}
]}
EOF

  rev_task="20261101-000008-inv2"
  mkdir -p "$AGENTS_HOME/tasks/$rev_task"
  cat > "$AGENTS_HOME/tasks/$rev_task/meta.env" << EOF
ID=$rev_task
AGENT=inv2
KIND=investigation
INVESTIGATION_TEAM=$id
REVIEW=1
STATUS=failed
EOF

  meta_put "$id" INVESTIGATION_PHASE 3
  meta_put "$id" INVESTIGATION_MERGER inv
  meta_put "$id" INVESTIGATION_REVIEWER inv2
  meta_put "$id" INVESTIGATION_REVIEWER_TASK "$rev_task"
  meta_put "$id" INVESTIGATION_REVIEW_RETRIES 0

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_REVIEW_RETRIES)" = "1" ]
  [ "$(field "$id" INVESTIGATION_REVIEWER)" = "inv3" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"review: inv3"* ]]
}

@test "second reviewer failure transitions parent task to waiting for human" {
  configure_three_investigators
  local base id dir c1 c2 c3 rev_task2
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000009-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"

  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  c3=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c3" "$base^{tree}")

  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"},
  {"agent":"inv3","task":"task-inv3","status":"done","commit":"$c3"}
]}
EOF

  rev_task2="20261101-000009-inv3"
  mkdir -p "$AGENTS_HOME/tasks/$rev_task2"
  cat > "$AGENTS_HOME/tasks/$rev_task2/meta.env" << EOF
ID=$rev_task2
AGENT=inv3
KIND=investigation
INVESTIGATION_TEAM=$id
REVIEW=1
STATUS=failed
EOF

  meta_put "$id" INVESTIGATION_PHASE 3
  meta_put "$id" INVESTIGATION_MERGER inv
  meta_put "$id" INVESTIGATION_REVIEWER inv3
  meta_put "$id" INVESTIGATION_REVIEWER_TASK "$rev_task2"
  meta_put "$id" INVESTIGATION_REVIEW_RETRIES 1

  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" STATUS)" = "failed" ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"failed: review failed"* ]]
}

@test "legacy parent without INVESTIGATION_PIPELINE finishes phase 1 as done without decision wait" {
  configure_three_investigators
  local base id dir c1 c2
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-000010-investigation-team"
  dir="$AGENTS_HOME/tasks/$id"
  create_parent_task "$id" 'inv,inv2' "$base"

  # Remove INVESTIGATION_PIPELINE to simulate legacy round-two parent task
  sed -i '/^INVESTIGATION_PIPELINE=/d' "$dir/meta.env"

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
  [ "$(field "$id" STATUS)" = "done" ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  [ "$(field "$id" AUTO_STAGE)" = "complete: phase 1 complete (merge not available yet)" ]
}

# orphan_role_recovery phase abandoned replacement checks interrupted admission and subsequent investigator reuse.
orphan_role_recovery() {
  local phase=$1 abandoned=$2 replacement=$3 base id child successor next
  configure_three_investigators
  scenario "{\"$replacement\":{\"new\":\"fail\"},\"$abandoned\":{\"new\":\"hold\"}}"
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  id="20261101-00003$phase-investigation-team"
  child="$id-abandoned"
  create_parent_task "$id" 'inv,inv2,inv3' "$base"
  printf '{"version":1,"members":[{"agent":"%s","status":"done","commit":"%s"},{"agent":"%s","status":"done","commit":"%s"}]}\n' \
    "$abandoned" "$base" "$replacement" "$base" > "$AGENTS_HOME/tasks/$id/phase1.json"
  mkdir -p "$AGENTS_HOME/tasks/$child"
  cat > "$AGENTS_HOME/tasks/$child/meta.env" <<META
ID=$child
AGENT=$abandoned
KIND=investigation
INVESTIGATION_TEAM=$id
INVESTIGATION_PHASE=$phase
STATUS=new
META
  meta_put "$id" INVESTIGATION_PHASE "$phase"
  if [[ $phase == 2 ]]; then
    meta_put "$id" INVESTIGATION_MERGER "$abandoned"
    meta_put "$id" INVESTIGATION_MERGER_TASK "$child"
  else
    meta_put "$id" INVESTIGATION_MERGER inv
    meta_put "$id" INVESTIGATION_REVIEWER "$abandoned"
    meta_put "$id" INVESTIGATION_REVIEWER_TASK "$child"
  fi
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$child" STATUS)" = failed ]
  if [[ $phase == 2 ]]; then successor=$(field "$id" INVESTIGATION_MERGER_TASK)
  else successor=$(field "$id" INVESTIGATION_REVIEWER_TASK); fi
  [ "$(field "$successor" AGENT)" = "$replacement" ]
  # The replacement also fails; the original investigator must remain reusable after handoff.
  wait_status "$id" failed 45
  [ "$(field "$successor" STATUS)" = failed ]
  [ "$(field "$id" STATUS)" = failed ]
  [ -z "$(field "$id" INVESTIGATION_LEASE)" ]
  next="$id-next"
  create_parent_task "$next" 'inv,inv2,inv3' "$base"
  meta_put "$next" INVESTIGATION_PHASE 2
  printf '{"version":1,"members":[{"agent":"%s","status":"done","commit":"%s"}]}\n' \
    "$abandoned" "$base" > "$AGENTS_HOME/tasks/$next/phase1.json"
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$next" INVESTIGATION_MERGER)" = "$abandoned" ]
  [ -n "$(field "$next" INVESTIGATION_MERGER_TASK)" ]
  touch "$HOME/release"
  wait_status "$next" failed 45
}

@test "unstarted merger is terminalized before replacement and its capacity is reusable" {
  orphan_role_recovery 2 inv inv2
}

@test "unstarted reviewer is terminalized before replacement and its capacity is reusable" {
  orphan_role_recovery 3 inv2 inv3
}
