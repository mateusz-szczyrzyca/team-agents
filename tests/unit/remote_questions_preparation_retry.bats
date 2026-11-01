#!/usr/bin/env bats
# Remote questions preparation must cleanly propagate errors from helper operations:
# if mktemp, awk, or copy fails, staging must be cleaned up, no partial or empty file
# should be published, and subsequent retries must be able to succeed.

load helpers

# setup_remote_question_fixture configures a waiting remote job fixture with mirrored questions
setup_remote_question_fixture() {
  echo 'LEAD_LISTEN=127.0.0.2:7780' >> "$AGENTS_HOME/agents.conf"
  mkdir -p "$AGENTS_HOME/bus/remote/node2/task-remote-1"
  cat > "$AGENTS_HOME/bus/remote/node2/task-remote-1/metadata.json" <<'META'
{"JOB_ID":"j-prep","AGENT":"worker-1"}
META
  cat > "$AGENTS_HOME/bus/remote/node2/task-remote-1/questions-1.md" <<'QMD'
# Questions — task-remote-1@node2 (round 1)

> Answer every question after **Verdict:** — `ok` accepts the recommendation. When you are done, run `agent answer task-remote-1@node2`.

## 1. Which model to select?
a) fast
b) precise

**Recommended:** a) — faster execution

**Verdict:**
QMD

  cat > "$AGENTS_HOME/bus/jobs.json" <<'JOBS'
{
  "Jobs": [
    {
      "Node": "node2",
      "TaskID": "task-remote-1",
      "State": "waiting",
      "Request": {
        "ID": "j-prep",
        "Repository": "team-agents"
      }
    }
  ]
}
JOBS
}

# create_hook_bin dir — creates interceptor wrappers for mktemp, awk, and cat in dir
create_hook_bin() {
  local dir=$1
  mkdir -p "$dir"

  # mktemp hook: tracks inner mktemp path and fails argument-free calls when FAIL_INNER_MKTEMP is set
  cat > "$dir/mktemp" <<'HOOK'
#!/usr/bin/env bash
if [[ "$#" -eq 0 ]]; then
  if [[ -n "${FAIL_INNER_MKTEMP:-}" ]]; then
    echo "mktemp-failed" > "$INJECT_FLAG"
    exit 1
  fi
  out=$(/usr/bin/mktemp) || exit 1
  echo "$out" > "$INNER_MKTEMP_LOG"
  echo "$out"
  exit 0
fi
exec /usr/bin/mktemp "$@"
HOOK
  chmod +x "$dir/mktemp"

  # awk hook: records injection and fails when FAIL_INNER_AWK is set on note invocation
  cat > "$dir/awk" <<'HOOK'
#!/usr/bin/env bash
for arg in "$@"; do
  if [[ "$arg" == *'note='* && -n "${FAIL_INNER_AWK:-}" ]]; then
    echo "awk-failed" > "$INJECT_FLAG"
    exit 1
  fi
done
exec /usr/bin/awk "$@"
HOOK
  chmod +x "$dir/awk"

  # cat hook: fails when FAIL_INNER_CAT is set and argument matches the tracked inner mktemp path
  cat > "$dir/cat" <<'HOOK'
#!/usr/bin/env bash
if [[ -n "${FAIL_INNER_CAT:-}" && -f "${INNER_MKTEMP_LOG:-}" ]]; then
  read -r inner < "$INNER_MKTEMP_LOG" || inner=""
  if [[ -n "$inner" && "${1:-}" == "$inner" ]]; then
    echo "cat-failed" > "$INJECT_FLAG"
    exit 1
  fi
fi
exec /bin/cat "$@"
HOOK
  chmod +x "$dir/cat"
}

@test "questions preparation fails and cleans up staging without publishing when inner mktemp fails, then retries cleanly" {
  setup_remote_question_fixture
  local remote_dir="$AGENTS_HOME/remote-questions/node2/task-remote-1"
  local target="$remote_dir/questions-1.md"
  local hook_bin="$BATS_TEST_TMPDIR/hook-bin"
  local inner_log="$BATS_TEST_TMPDIR/inner_mktemp.log"
  local flag="$BATS_TEST_TMPDIR/inject.flag"
  create_hook_bin "$hook_bin"

  run env FAIL_INNER_MKTEMP=1 INNER_MKTEMP_LOG="$inner_log" INJECT_FLAG="$flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"
  [ "$status" -ne 0 ]
  [ -f "$flag" ]
  [ ! -e "$target" ]
  local leaked
  leaked=$(find "$remote_dir" -maxdepth 1 -name '.questions-*')
  [ -z "$leaked" ]

  run env INNER_MKTEMP_LOG="$inner_log" INJECT_FLAG="$flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"
  [ "$status" -eq 0 ]
  [ -f "$target" ]
  [ "$(stat -c %a "$target")" = "600" ]
  grep -qF '> Answer every question after **Verdict:** — `ok` accepts the recommendation.' "$target"
  grep -qF '## 1. Which model to select?' "$target"
}

@test "questions preparation fails and cleans up staging without publishing when inner awk fails, then retries cleanly" {
  setup_remote_question_fixture
  local remote_dir="$AGENTS_HOME/remote-questions/node2/task-remote-1"
  local target="$remote_dir/questions-1.md"
  local hook_bin="$BATS_TEST_TMPDIR/hook-bin"
  local inner_log="$BATS_TEST_TMPDIR/inner_mktemp.log"
  local flag="$BATS_TEST_TMPDIR/inject.flag"
  create_hook_bin "$hook_bin"

  run env FAIL_INNER_AWK=1 INNER_MKTEMP_LOG="$inner_log" INJECT_FLAG="$flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"
  [ "$status" -ne 0 ]
  [ -f "$flag" ]
  [ ! -e "$target" ]
  local leaked
  leaked=$(find "$remote_dir" -maxdepth 1 -name '.questions-*')
  [ -z "$leaked" ]
  local inner_tmp
  read -r inner_tmp < "$inner_log"
  [ -n "$inner_tmp" ]
  [ ! -e "$inner_tmp" ]

  rm -f "$flag"
  run env INNER_MKTEMP_LOG="$inner_log" INJECT_FLAG="$flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"
  [ "$status" -eq 0 ]
  [ -f "$target" ]
  [ "$(stat -c %a "$target")" = "600" ]
  grep -qF '> Answer every question after **Verdict:** — `ok` accepts the recommendation.' "$target"
  grep -qF '## 1. Which model to select?' "$target"
}

@test "questions preparation fails and cleans up staging without publishing when inner cat copy fails, then retries cleanly" {
  setup_remote_question_fixture
  local remote_dir="$AGENTS_HOME/remote-questions/node2/task-remote-1"
  local target="$remote_dir/questions-1.md"
  local hook_bin="$BATS_TEST_TMPDIR/hook-bin"
  local inner_log="$BATS_TEST_TMPDIR/inner_mktemp.log"
  local flag="$BATS_TEST_TMPDIR/inject.flag"
  create_hook_bin "$hook_bin"

  run env FAIL_INNER_CAT=1 INNER_MKTEMP_LOG="$inner_log" INJECT_FLAG="$flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"
  [ "$status" -ne 0 ]
  [ -f "$flag" ]
  [ ! -e "$target" ]
  local leaked
  leaked=$(find "$remote_dir" -maxdepth 1 -name '.questions-*')
  [ -z "$leaked" ]
  local inner_tmp
  read -r inner_tmp < "$inner_log"
  [ -n "$inner_tmp" ]
  [ ! -e "$inner_tmp" ]

  rm -f "$flag"
  run env INNER_MKTEMP_LOG="$inner_log" INJECT_FLAG="$flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"
  [ "$status" -eq 0 ]
  [ -f "$target" ]
  [ "$(stat -c %a "$target")" = "600" ]
  grep -qF '> Answer every question after **Verdict:** — `ok` accepts the recommendation.' "$target"
  grep -qF '## 1. Which model to select?' "$target"
}
