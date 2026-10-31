#!/usr/bin/env bats
# Remote waiting notifications keep their links usable with read-only or interactive frontends.
load helpers

@test "remote question notifications link to the task view and qualified answer command" {
  # Load the script inside the isolated test HOME; no real notification service runs.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  export LEAD_LISTEN=127.0.0.1:1 AGENT_WEB_URL=https://example.com
  mkdir -p "$BUS_DIR/remote/node/T1" "$AGENTS_HOME/remote-questions/node/T1"
  printf '{"AGENT":"solo","TYPE":"opencode","MODEL":"fake","STATUS":"waiting","RUN":"1","JOB_ID":"j1"}\n' > "$BUS_DIR/remote/node/T1/metadata.json"
  printf '{"Jobs":[{"Request":{"ID":"j1","Repository":"team-agents"},"Node":"node","TaskID":"T1","State":"waiting"}]}\n' > "$BUS_DIR/jobs.json"
  printf '# Questions\n\n## 1. Proceed?\n**Verdict:**\n' > "$BUS_DIR/remote/node/T1/questions-1.md"
  cp "$BUS_DIR/remote/node/T1/questions-1.md" "$AGENTS_HOME/remote-questions/node/T1/questions-1.md"
  # notify_send exposes the rendered message without contacting an external service.
  notify_send() { cat "$1"; }
  run notify T1@node waiting "Questions file: $AGENTS_HOME/remote-questions/node/T1/questions-1.md"
  [ "$status" -eq 0 ]
  [[ $output == *'agent answer T1@node'* ]]
  [[ $output == *'Open: https://example.com/task/T1@node'* ]]
  [[ $output != *'/questions/T1@node'* ]]
}
