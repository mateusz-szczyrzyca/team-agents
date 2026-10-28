#!/usr/bin/env bats
# Delegated submissions preserve origin attribution through the daemon boundary.

load helpers

# attribution_path_functions extracts only the daemon and origin boundaries under test.
attribution_path_functions() {
  local source="$BATS_TEST_DIRNAME/../../bin/agent"
  sed -n '/^github_repo_url() {/,/^}/p' "$source" > "$HOME/attribution-paths.sh"
  sed -n '/^node_poll_remote_helpers() {/,/^}/p' "$source" >> "$HOME/attribution-paths.sh"
}

@test "repository attribution uses the first origin URL like existing GitHub operations" {
  attribution_path_functions
  git remote set-url origin https://github.com/first/repo.git
  git config --add remote.origin.url https://github.com/second/repo.git
  run bash -c 'source "$HOME/attribution-paths.sh"; log_warn() { echo "$*" >&2; }; github_repo_url "$HOME/repo"'
  [ "$status" -eq 0 ]
  [ "$output" = https://github.com/first/repo ]
}

@test "the node passes normalized origin attribution into frozen helper preparation" {
  attribution_path_functions
  git remote set-url origin ssh://git@github.com/acme/repo.git
  run bash -c '
    set -euo pipefail
    source "$HOME/attribution-paths.sh"
    BUS_DIR="$HOME/bus"; INSTANCE_NAME=node-a; INSTANCE_CAPABILITIES=()
    log_warn() { echo "$*" >&2; }
    bus() {
      case $1 in
        helper-remote-list) jq -nc --arg wt "$HOME/repo" "[{id:1,worktree:\$wt,status:\"queued\"}]" ;;
        helper-remote-prepare)
          printf "%s\n" "$@" > "$HOME/prepare-arguments"
          echo "{\"ID\":\"job-1\"}"
          ;;
        *) echo "unexpected bus operation $1" >&2; return 1 ;;
      esac
    }
    node_request() { return 5; }
    node_poll_remote_helpers
  '
  [ "$status" -eq 0 ]
  grep -Fx -- --repo-url "$HOME/prepare-arguments"
  grep -Fx https://github.com/acme/repo "$HOME/prepare-arguments"
}
