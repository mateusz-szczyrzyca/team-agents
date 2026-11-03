#!/usr/bin/env bash
# check-agent-parity.sh — validates parity ledger and candidate execution evidence.
set -euo pipefail

# SCRIPT_DIR locates the ledger validator independently of the selected fixture root.
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
# ROOT is the checkout under validation; CHECK_ROOT supports isolated harness fixtures.
ROOT=${CHECK_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}
# QUIET suppresses successful validation output.
QUIET=0
# EVIDENCE is the candidate runner's machine-readable execution record.
EVIDENCE="$ROOT/build/agent-bats-evidence.json"
# EXPLICIT_EVIDENCE makes a missing explicitly requested record an error even without selections.
EXPLICIT_EVIDENCE=0

# err message — report a validation failure and stop without accepting incomplete evidence.
err() {
  echo "check-agent-parity: $*" >&2
  exit 1
}

while (( $# > 0 )); do
  case "$1" in
    -q|--quiet) QUIET=1; shift ;;
    --evidence)
      (( $# >= 2 )) || err "--evidence requires a path"
      EVIDENCE=$2
      EXPLICIT_EVIDENCE=1
      shift 2
      ;;
    *) err "unknown option: $1" ;;
  esac
done

CHECK_ROOT="$ROOT" "$SCRIPT_DIR/check-agent-test-map.sh" --quiet

# ACTUAL holds the dynamically derived Bats identities for reverse ledger membership checks.
declare -A ACTUAL=()
# file is each unchanged Bats source under the selected root.
for file in "$ROOT"/tests/unit/*.bats; do
  # title is a literal Bats title, never a regular expression.
  while IFS= read -r title; do
    ACTUAL["tests/unit/${file##*/}"$'\t'"$title"]=1
  done < <(sed -n 's/^@test "\([^"]*\)".*/\1/p' "$file")
done

# validate_mapping mapping — require real top-level Go declarations for activated contracts.
validate_mapping() {
  local mapping=$1
  local token top
  local -a tokens=() go_files=("$ROOT"/agent/*_test.go)
  [[ $mapping != - && $mapping != ,* && $mapping != *, && $mapping != *,,* ]] || err "Go test mapping required: $mapping"
  [[ -f ${go_files[0]} ]] || err "Go test mapping has no agent test sources: $mapping"
  IFS=',' read -r -a tokens <<< "$mapping"
  for token in "${tokens[@]}"; do
    [[ $token =~ ^Test([A-Z0-9_][A-Za-z0-9_]*)?(/[^,]+)?$ ]] || err "invalid Go test mapping: $token"
    top=${token%%/*}
    # grep returning one is the expected absence of a declared mapped test.
    if grep -Eq "^[[:space:]]*func[[:space:]]+${top}[[:space:]]*\\(" "${go_files[@]}"; then
      continue
    fi
    err "Go test mapping not declared in agent: $token"
  done
}

# Counts summarize validated classifications and lifecycle states, with selected candidate rows separate.
blackbox=0 source=0 planned=0 active=0 done_count=0 selected=0
# Each ledger identity must exist even when aggregate map and suite counts coincide.
while IFS=$'\t' read -r file title classification _round mapping state; do
  [[ $file != bats_file ]] || continue
  [[ -n ${ACTUAL["$file"$'\t'"$title"]:-} ]] || err "ledger identity not found in Bats: $file :: $title"
  case "$classification" in
    blackbox) blackbox=$((blackbox + 1)) ;;
    source) source=$((source + 1)) ;;
  esac
  case "$state" in
    planned) planned=$((planned + 1)) ;;
    active) active=$((active + 1)); validate_mapping "$mapping" ;;
    done) done_count=$((done_count + 1)); validate_mapping "$mapping" ;;
  esac
  if [[ $classification == blackbox && $state != planned ]]; then
    selected=$((selected + 1))
  fi
done < "$ROOT/tests/agent-go-map.tsv"

if [[ -f $EVIDENCE ]]; then
  # Only integral counts of successfully executed, unskipped candidate cases prove parity.
  if ! jq -e --argjson expected "$selected" '
    type == "object" and .lane == "candidate" and
    ([.selected_count, .executed_count, .failures, .skips] |
      all(.[]; type == "number" and . >= 0 and . == floor)) and
    .selected_count == $expected and .executed_count == $expected and
    .failures == 0 and .skips == 0
  ' "$EVIDENCE" > /dev/null; then
    err "invalid candidate evidence or execution counts: $EVIDENCE"
  fi
  # map_hash binds the execution evidence to this exact ledger snapshot.
  map_hash=$(sha256sum "$ROOT/tests/agent-go-map.tsv")
  map_hash=${map_hash%% *}
  # tests_hash covers Bats sources, helpers, and stubs using root-relative names.
  tests_hash=$(cd "$ROOT" && find tests/unit -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum)
  tests_hash=${tests_hash%% *}
  if ! jq -e --arg map "$map_hash" --arg tests "$tests_hash" \
    '.map_sha256 == $map and .tests_sha256 == $tests' "$EVIDENCE" > /dev/null; then
    err "evidence hash does not match ledger or test tree: $EVIDENCE"
  fi
elif (( selected > 0 || EXPLICIT_EVIDENCE )); then
  err "candidate evidence not found: $EVIDENCE"
fi
# With no activated black-box cases, a fresh checkout has no candidate execution to prove.
if (( ! QUIET )); then
  echo "check-agent-parity: OK blackbox=$blackbox source=$source planned=$planned active=$active done=$done_count selected=$selected"
fi
