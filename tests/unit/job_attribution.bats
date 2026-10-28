#!/usr/bin/env bats
# Submission link data comes from a GitHub origin, never an arbitrary remote URL.

load helpers

# attribution_functions extracts the submission boundary with isolated bus/config fakes.
attribution_functions() {
  local source="$BATS_TEST_DIRNAME/../../bin/agent"
  sed -n '/^submit_remote_job() {/,/^}/p' "$source" > "$HOME/attribution.sh"
  sed -n '/^github_repo_url() {/,/^}/p' "$source" >> "$HOME/attribution.sh"
}

@test "remote submissions attach canonical GitHub origin and no human requester task" {
  attribution_functions
  git remote set-url origin git@github.com:acme/repo.git
  run bash -c '
    set -euo pipefail
    source "$HOME/attribution.sh"
    die() { echo "$*" >&2; exit 1; }
    lead_url() { echo http://127.0.0.1:7780; }
    log_warn() { echo "$*" >&2; }
    bus() { cat > "$HOME/submission.json"; echo "{\"State\":\"queued\"}"; }
    submit_remote_job worker node-a "work"
  '
  [ "$status" -eq 0 ]
  [ "$(jq -r '.RepoURL' "$HOME/submission.json")" = 'https://github.com/acme/repo' ]
  [ "$(jq -r '.RequesterTask' "$HOME/submission.json")" = '' ]
}

@test "non-GitHub origins do not become repository links" {
  attribution_functions
  git remote set-url origin https://example.com/acme/repo.git
  run bash -c '
    set -euo pipefail
    source "$HOME/attribution.sh"
    die() { echo "$*" >&2; exit 1; }
    lead_url() { echo http://127.0.0.1:7780; }
    log_warn() { echo "$*" >&2; }
    bus() { cat > "$HOME/submission.json"; echo "{\"State\":\"queued\"}"; }
    submit_remote_job worker node-a "work"
  '
  [ "$status" -eq 0 ]
  [ "$(jq -r '.RepoURL' "$HOME/submission.json")" = '' ]
}
