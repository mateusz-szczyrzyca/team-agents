#!/usr/bin/env bats
# Runs that start at the same moment (the two reviews of a pair do) each put the agents' gh in front of the real one:
# the link run/gh-trusted/gh is replaced at once, never missing, so no run dies with "the agents' gh … could not be
# set up". A slow ln — as on a loaded machine: a forced link's old one removed, a pause, the new one made — makes
# the moment the same for every run.

load helpers

@test "runs setting up the agents' gh at once all succeed, and the link is the script" {
  local i
  for i in $(seq 1 6); do
    ( source "$REPO_ROOT/bin/agent"
      # ln, slowly: with -f the target removed, a pause, then linked without -f (what ln -f does in one go)
      ln() {
        local args=() force=0 a
        for a in "$@"; do
          if [[ $a == -* && $a == *f* ]]; then force=1; a=${a//f/}; [[ $a != - ]] || continue; fi
          args+=("$a")
        done
        if ((force)); then rm -f "${args[-1]}"; sleep 0.3; fi
        command ln "${args[@]}"
      }
      RUNNER_ID="" REPO_ROOT=$HOME/repo AGENT_GITHUB_TRUSTED=o agents_gh_env || touch "$BATS_TEST_TMPDIR/failed-$i" ) &
  done
  wait
  run compgen -G "$BATS_TEST_TMPDIR/failed-*"
  [ "$status" -ne 0 ]
  [ -L "$AGENTS_HOME/run/gh-trusted/gh" ] && [ -x "$AGENTS_HOME/run/gh-trusted/gh" ]
}
