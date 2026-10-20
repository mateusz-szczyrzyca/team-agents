# Shared setup for the bats tests of the `agent` script. Everything runs in a temporary HOME with stub
# CLIs: no real agent CLI, systemd or GitHub is touched.

REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)

setup_file() {
  export BUILD_DIR="$BATS_FILE_TMPDIR/bin"
  mkdir -p "$BUILD_DIR"
  (cd "$REPO_ROOT/agentbus" && go build -o "$BUILD_DIR/agentbus" .)
}

setup() {
  export HOME="$BATS_TEST_TMPDIR/home"
  export STUB_HOME="$HOME"
  export STUB_UNITS="$HOME/units"
  export AGENTS_HOME="$HOME/agents"
  mkdir -p "$HOME/.local/bin" "$STUB_UNITS" "$AGENTS_HOME"/{tasks,worktrees,bus,prompts}
  install -m755 "$REPO_ROOT/bin/agent" "$HOME/.local/bin/agent"
  cp "$BUILD_DIR/agentbus" "$HOME/.local/bin/agentbus"
  cp "$BATS_TEST_DIRNAME/stubs/fake-claude" "$HOME/.local/bin/claude"
  cp "$REPO_ROOT"/prompts/*.md "$AGENTS_HOME/prompts/"
  export PATH="$BATS_TEST_DIRNAME/stubs:$HOME/.local/bin:$PATH"

  cat > "$HOME/notify.sh" <<'NOTIFY'
#!/usr/bin/env bash
{ echo "=== $(basename "$1")"; cat "$1"; } >> "$STUB_HOME/notifications.log"
NOTIFY
  chmod +x "$HOME/notify.sh"

  cat > "$AGENTS_HOME/agents.conf" <<CONF
declare -A AGENT_TYPE=([solo]=claude [lead]=claude [member]=claude [worker]=claude [worker2]=claude)
declare -A AGENT_MODEL=()
NORMAL_AGENTS=(solo lead member)
WORKER_AGENTS=(worker worker2)
declare -A INVESTIGATORS_TYPE=([inv]=claude)
CLAUDE_BIN="$HOME/.local/bin/claude"
RUN_PATH="$BATS_TEST_DIRNAME/stubs:$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin"
NOTIFY_CMD="$HOME/notify.sh"
AGENT_HANG_SECS=600
AGENT_HANG_POLL=1
CONF
  : > "$AGENTS_HOME/secrets.env"

  git init -q --bare "$HOME/remote.git"
  git init -q -b main "$HOME/repo"
  cd "$HOME/repo"
  git -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
  git remote add origin "$HOME/remote.git"
  git push -q origin main
  echo '{}' > "$HOME/scenario.json"
}

teardown() {
  # kill anything still running from this test
  for p in "$STUB_UNITS"/*.pid; do
    [[ -f $p ]] && kill -TERM -- -"$(cat "$p")" 2>/dev/null || true
  done
}

# scenario '{"agent": {"new": "action", "resume": "action"}}'
scenario() { echo "$1" > "$HOME/scenario.json"; }

# task_of <agent> — the newest task ID of an agent
task_of() { ls -t "$AGENTS_HOME/tasks" | grep -- "-$1\$" | head -1; }

# status_of <task id>
status_of() { sed -n 's/^STATUS=//p' "$AGENTS_HOME/tasks/$1/meta.env"; }

# wait_status <task id> <regex> [timeout seconds]
wait_status() {
  local t=$1 re=$2 n=${3:-30} st
  for _ in $(seq 1 $((n * 5))); do
    st=$(status_of "$t")
    [[ $st =~ ^($re)$ ]] && return 0
    sleep 0.2
  done
  echo "status of $t is '$st', expected /$re/" >&2
  return 1
}

# wait_grep pattern file [timeout seconds] — waits until the file contains the pattern (fixed string)
wait_grep() {
  local n=${3:-15}
  for _ in $(seq 1 $((n * 5))); do
    [[ -f $2 ]] && grep -qF -- "$1" "$2" && return 0
    sleep 0.2
  done
  echo "'$1' not found in $2" >&2
  return 1
}
