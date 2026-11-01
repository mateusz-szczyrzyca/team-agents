#!/usr/bin/env bats
# Remote questions must be staged and published atomically: no reader or watcher
# should ever observe the destination file before permissions and instructions are complete.

load helpers

@test "remote questions are staged and published atomically with instructions and permissions" {
  echo 'LEAD_LISTEN=127.0.0.2:7780' >> "$AGENTS_HOME/agents.conf"
  mkdir -p "$AGENTS_HOME/bus/remote/node2/task-remote-1"
  cat > "$AGENTS_HOME/bus/remote/node2/task-remote-1/metadata.json" <<'META'
{"JOB_ID":"j-atomic","AGENT":"worker-1"}
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
        "ID": "j-atomic",
        "Repository": "team-agents"
      }
    }
  ]
}
JOBS

  local target="$AGENTS_HOME/remote-questions/node2/task-remote-1/questions-1.md"
  local hook_bin="$BATS_TEST_TMPDIR/hook-bin"
  mkdir -p "$hook_bin"

  # mktemp is called by questions_note while constructing the instruction banner.
  # If the destination file already exists at this point, publication is unatomic.
  cat > "$hook_bin/mktemp" <<'HOOK'
#!/usr/bin/env bash
if [[ -e "$TARGET_FILE" ]]; then
  echo "target exists before questions_note completed" > "$FLAG_FILE"
fi
exec /bin/mktemp "$@"
HOOK
  chmod +x "$hook_bin/mktemp"

  TARGET_FILE="$target" FLAG_FILE="$BATS_TEST_TMPDIR/unatomic-mktemp.flag" \
    PATH="$hook_bin:$PATH" agent _remote_questions_ready "task-remote-1@node2"

  [ ! -f "$BATS_TEST_TMPDIR/unatomic-mktemp.flag" ]
  [ -f "$target" ]
  [ "$(stat -c %a "$target")" = "600" ]
  grep -qF '> Answer every question after **Verdict:** — `ok` accepts the recommendation. When you are done, put **ANSWERED** on the last line and save: the answers are sent at once (or run `agent answer task-remote-1@node2`).' "$target"
  grep -qF '## 1. Which model to select?' "$target"

  local path_unit="$STUB_UNITS/agent-task-remote-1@node2-answers-1.path"
  [ -f "$path_unit" ]
  grep -qF "PathModified=$target" "$path_unit"
}
