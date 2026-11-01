#!/usr/bin/env bats
# Publication transport errors must not turn a completed independent result into a permanent failure.
load helpers

# publication_field reads field $2 from task $1 without inheriting the caller's metadata.
publication_field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# publication_cohort creates a completed child and a leased parent with a real local publication.
publication_cohort() {
  local base commit
  base=$(git rev-parse HEAD)
  printf '%s\n' findings > findings.md
  git add findings.md
  git -c user.name=t -c user.email=t@example.com commit -qm findings
  git push -q origin main
  commit=$(git rev-parse HEAD)
  export PUBLICATION_PARENT=20261101-000000-investigation-team PUBLICATION_CHILD=20261101-000000-inv
  mkdir -p "$AGENTS_HOME/tasks/$PUBLICATION_PARENT" "$AGENTS_HOME/tasks/$PUBLICATION_CHILD"
  cat > "$AGENTS_HOME/tasks/$PUBLICATION_PARENT/meta.env" <<EOF
ID=$PUBLICATION_PARENT
AGENT=investigation-team
KIND=investigation_team
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
STATUS=running
RUN=0
RUNNER_DONE=1
INVESTIGATION_PHASE=1
INVESTIGATION_LEASE=1
INVESTIGATION_TEAM=inv
INVESTIGATION_BASE_SHA=$base
PR_URL=
EOF
  cat > "$AGENTS_HOME/tasks/$PUBLICATION_CHILD/meta.env" <<EOF
ID=$PUBLICATION_CHILD
AGENT=inv
KIND=investigation
STATUS=done
RUN=1
RUNNER_DONE=1
INVESTIGATION_TEAM=$PUBLICATION_PARENT
WORKTREE=$HOME/repo
BRANCH=main
EOF
  jq -n --arg task "$PUBLICATION_CHILD" '{version:1,members:[{agent:"inv",task:$task,branch:"main",status:"new",commit:""}]}' \
    > "$AGENTS_HOME/tasks/$PUBLICATION_PARENT/phase1.json"
  [ -n "$commit" ]
}

# publication_transport_failure installs a git wrapper that fails only remote publication lookups until released.
publication_transport_failure() {
  local real_git
  real_git=$(command -v git)
  export PUBLICATION_REAL_GIT="$real_git"
  touch "$HOME/publication-offline"
  cat > "$HOME/.local/bin/git" <<'GIT'
#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
  if [[ $argument == ls-remote && -f $HOME/publication-offline ]]; then
    touch "$HOME/publication-lookup-attempted"
    printf '%s\n' 'temporary publication transport failure' >&2
    exit 128
  fi
done
exec "$PUBLICATION_REAL_GIT" "$@"
GIT
  chmod +x "$HOME/.local/bin/git"
}

@test "publication transport failure preserves DONE and retries until the published branch can be verified" {
  publication_cohort
  publication_transport_failure
  run agent _supervise
  [ "$status" -eq 0 ]
  [ -f "$HOME/publication-lookup-attempted" ]
  [ "$(status_of "$PUBLICATION_CHILD")" = 'done' ]
  [ "$(status_of "$PUBLICATION_PARENT")" = running ]
  [ "$(publication_field "$PUBLICATION_PARENT" INVESTIGATION_LEASE)" = 1 ]
  [ "$(jq -r '.members[0].status' "$AGENTS_HOME/tasks/$PUBLICATION_PARENT/phase1.json")" != failed ]
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$PUBLICATION_CHILD")" = 'done' ]
  rm "$HOME/publication-offline"
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$PUBLICATION_PARENT")" = waiting ]
  [ "$(jq -r '.members[0].status' "$AGENTS_HOME/tasks/$PUBLICATION_PARENT/phase1.json")" = 'done' ]
  [ "$(jq -r '.members[0].commit' "$AGENTS_HOME/tasks/$PUBLICATION_PARENT/phase1.json")" = "$(git rev-parse HEAD)" ]
}

@test "a successful lookup confirming a missing published branch still fails the independent result" {
  publication_cohort
  sed -i 's/^BRANCH=main$/BRANCH=unpublished/' "$AGENTS_HOME/tasks/$PUBLICATION_CHILD/meta.env"
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$PUBLICATION_CHILD")" = failed ]
  [ "$(status_of "$PUBLICATION_PARENT")" = waiting ]
  [ "$(jq -r '.members[0].status' "$AGENTS_HOME/tasks/$PUBLICATION_PARENT/phase1.json")" = failed ]
}
