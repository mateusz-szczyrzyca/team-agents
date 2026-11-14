#!/usr/bin/env bash
set -euo pipefail
export GIT_AUTHOR_NAME='Fixture Author' GIT_AUTHOR_EMAIL='fixture@example.com'
export GIT_COMMITTER_NAME='Fixture Author' GIT_COMMITTER_EMAIL='fixture@example.com'
printf 'session fixture\n' > session-change.txt
git add session-change.txt
git -c commit.gpgsign=false commit -qm 'Record fixture work'
fixture_home=""
while (($#)); do
  case $1 in
    --home) fixture_home=$2; shift ;;
  esac
  shift
done
head=$(git rev-parse HEAD)
task=${PWD##*/}
dir="$fixture_home/tasks/$task"
printf '{"tree":"clean","result":{"head":"%s","finished_at":"2026-11-13T10:00:00Z","outcome":{"kind":"passed"},"log":"%s/gate-1.log"}}\n' "$head" "$dir" > "$dir/gate.json"
printf '%s\n' \
  '{"type":"step_start","sessionID":"ses-first"}' \
  '{"type":"text","sessionID":"ses-first","part":{"text":"working"}}' \
  '{"type":"text","sessionID":"ses-second","part":{"text":"STATUS: DONE"}}' \
  '{"type":"step_finish","sessionID":"ses-second","part":{"reason":"stop"}}'
