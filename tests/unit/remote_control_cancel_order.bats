#!/usr/bin/env bats
# Cancellation must be applied before queued controls in the same heartbeat.
load helpers

@test "node processes cancellation before an answer from the same heartbeat" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # These isolated adapters expose ordering without launching a runner or systemd unit.
  node_handle_stop() { printf '%s' "$1" > "$HOME/cancelled"; }
  node_apply_answer() {
    [[ -f $HOME/cancelled ]] || return 1
    printf '%s' "$1" > "$HOME/control-after-cancel"
  }
  node_handle_reply '{"Stops":[{"Request":{"ID":"job-cancel"},"Assignment":1}],"Answers":[{}],"Kicks":[],"Wakes":[],"Job":null}'
  [ -f "$HOME/control-after-cancel" ]
  [ "$(cat "$HOME/cancelled")" = job-cancel ]
}
